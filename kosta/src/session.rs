// kosta/src/session.rs

//! Клиентская сессия для KoSta протокола.
//! Обеспечивает полный цикл аутентификации, шифрование сообщений и работу с TL-объектами.

use kosta_core::tl::types::{
    Int128, Int256, TlWrite, TlRead,
    MessageId, ChatId, UserId, UserStatus,
};
use kosta_core::tl::constructors::*;
use kosta_crypto::session_crypto::SessionCrypto;
use kosta_crypto::AuthKey;
use kosta_transport::{Transport, StreamTransport, tls, MockTransport};
use crate::error::KostaError;
use rand::rngs::OsRng;
use rand::Rng;
use std::net::TcpStream;
use std::io::Cursor;
use num_bigint::BigUint;
use sha1::{Sha1, Digest};
use crate::server_keys::{ServerKeypair, get_server_public_key};
use std::time::{SystemTime, UNIX_EPOCH, Duration};
use tracing::{info, debug};

// =============================================================================
// Вспомогательные функции
// =============================================================================

/// Вычисляет отпечаток публичного ключа (первые 64 бита SHA-1)
fn compute_key_fingerprint(public_key: &ed25519_dalek::VerifyingKey) -> i64 {
    let mut hasher = Sha1::new();
    hasher.update(public_key.as_bytes());
    let hash = hasher.finalize();
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&hash[12..20]);
    i64::from_le_bytes(buf)
}

/// Вычисляет auth_key_id как первые 64 бита SHA-1 от общего секрета
fn compute_auth_key_id(shared_secret: &BigUint) -> i64 {
    let secret_bytes = shared_secret.to_bytes_le();
    let mut hasher = Sha1::new();
    hasher.update(&secret_bytes);
    let hash = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&hash[12..20]);
    i64::from_le_bytes(bytes)
}

// =============================================================================
// Основная структура Session
// =============================================================================

/// Клиентская сессия, управляющая подключением, аутентификацией и обменом сообщениями.
pub struct Session<T: Transport> {
    pub auth_key_id: Option<i64>,
    pub crypto: Option<SessionCrypto>,
    pub server_salt: i64,
    pub session_id: i64,
    pub last_msg_id: i64,
    pub recv_last_msg_id: i64,
    pub seq_no: i32,
    pub transport: T,
    pub auth_key: Option<AuthKey>,
    pub recv_seq_no: i32,
    pub msg_id_counter: u32,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user_id: Option<UserId>,
    pub use_tls: bool,
    pub tls_domain: Option<String>,
    pub timeout: Duration,
}

impl Session<Box<dyn Transport>> {
    /// Создаёт новое соединение с сервером.
    ///
    /// # Аргументы
    /// * `host` – адрес сервера
    /// * `port` – порт
    /// * `use_tls` – использовать TLS
    /// * `tls_domain` – домен для TLS (если требуется)
    /// * `timeout_secs` – таймаут операций в секундах (0 – без таймаута)
    pub fn connect(
        host: &str,
        port: u16,
        use_tls: bool,
        tls_domain: &str,
        timeout_secs: u64,
    ) -> Result<Self, KostaError> {
        let transport: Box<dyn Transport> = if use_tls {
            let tls_stream = tls::connect_tls(host, port, tls_domain)
                .map_err(|e| KostaError::ConnectionFailed(format!("TLS connection failed: {}", e)))?;
            Box::new(tls_stream)
        } else {
            let stream = TcpStream::connect((host, port))?;
            // Устанавливаем таймауты, если заданы
            if timeout_secs > 0 {
                stream.set_read_timeout(Some(Duration::from_secs(timeout_secs)))?;
                stream.set_write_timeout(Some(Duration::from_secs(timeout_secs)))?;
            }
            Box::new(StreamTransport::new(stream))
        };
        let mut rng = OsRng;
        let session_id: i64 = rng.gen();
        Ok(Session {
            auth_key_id: None,
            crypto: None,
            server_salt: 0,
            session_id,
            last_msg_id: 0,
            recv_last_msg_id: 0,
            seq_no: -1,
            transport,
            auth_key: None,
            recv_seq_no: -1,
            msg_id_counter: 0,
            host: Some(host.to_string()),
            port: Some(port),
            user_id: None,
            use_tls,
            tls_domain: Some(tls_domain.to_string()),
            timeout: if timeout_secs > 0 {
                Duration::from_secs(timeout_secs)
            } else {
                Duration::from_secs(30) // разумный дефолт
            },
        })
    }

    /// Создаёт сессию из готового транспорта (для тестов или встраивания).
    pub fn from_transport(transport: StreamTransport<TcpStream>, timeout_secs: u64) -> Self {
        let mut rng = OsRng;
        Session {
            auth_key_id: None,
            crypto: None,
            server_salt: 0,
            session_id: rng.gen(),
            last_msg_id: 0,
            recv_last_msg_id: 0,
            seq_no: -1,
            transport: Box::new(transport),
            auth_key: None,
            recv_seq_no: -1,
            msg_id_counter: 0,
            host: None,
            port: None,
            user_id: None,
            use_tls: false,
            tls_domain: None,
            timeout: if timeout_secs > 0 {
                Duration::from_secs(timeout_secs)
            } else {
                Duration::from_secs(30)
            },
        }
    }

    /// Переподключается к серверу, сбрасывая все состояния сессии (кроме идентификатора сессии).
    ///
    /// После вызова `authenticate()` должна быть вызвана заново.
    pub fn reconnect(&mut self) -> Result<(), KostaError> {
        if let (Some(host), Some(port)) = (self.host.as_ref(), self.port) {
            let use_tls = self.use_tls;
            let domain = self.tls_domain.as_deref().unwrap_or(host);
            let transport: Box<dyn Transport> = if use_tls {
                let tls_stream = tls::connect_tls(host, port, domain)
                    .map_err(|e| KostaError::ConnectionFailed(format!("TLS reconnect failed: {}", e)))?;
                Box::new(tls_stream)
            } else {
                let stream = TcpStream::connect((host.as_str(), port))?;
                if self.timeout.as_secs() > 0 {
                    stream.set_read_timeout(Some(self.timeout))?;
                    stream.set_write_timeout(Some(self.timeout))?;
                }
                Box::new(StreamTransport::new(stream))
            };
            self.transport = transport;
            // Сброс состояний (кроме auth_key_id, crypto и т.п. – они будут пересозданы при authenticate)
            self.auth_key_id = None;
            self.crypto = None;
            self.server_salt = 0;
            self.last_msg_id = 0;
            self.recv_last_msg_id = 0;
            self.seq_no = -1;
            self.auth_key = None;
            self.recv_seq_no = -1;
            self.msg_id_counter = 0;
            self.user_id = None;
            Ok(())
        } else {
            Err(KostaError::ConnectionFailed("No host/port stored for reconnect".into()))
        }
    }
}

impl Session<MockTransport> {
    /// Создаёт сессию из MockTransport (для тестов).
    pub fn from_mock_transport(transport: MockTransport) -> Self {
        let mut rng = OsRng;
        Session {
            auth_key_id: None,
            crypto: None,
            server_salt: 0,
            session_id: rng.gen(),
            last_msg_id: 0,
            recv_last_msg_id: 0,
            seq_no: -1,
            transport,
            auth_key: None,
            recv_seq_no: -1,
            msg_id_counter: 0,
            host: None,
            port: None,
            user_id: None,
            use_tls: false,
            tls_domain: None,
            timeout: Duration::from_secs(30),
        }
    }
}

impl<T: Transport> Session<T> {
    // -------------------------------------------------------------------------
    // Публичные методы
    // -------------------------------------------------------------------------

    /// Выполняет полный цикл аутентификации (обмен ключами) с сервером.
    /// При успехе заполняет `auth_key_id`, `crypto`, `auth_key` и другие поля.
    pub fn authenticate(&mut self) -> Result<(), KostaError> {
        let mut rng = OsRng;

        // --- шаг 1: req_pq ---
        let mut nonce_bytes = [0u8; 16];
        rng.fill(&mut nonce_bytes);
        let nonce = Int128(nonce_bytes);
        info!("Sending ReqPq");
        self.send_tl(&TlObject::ReqPq(ReqPq { nonce: nonce.clone() }))?;

        // --- шаг 2: ResPQ ---
        let resp = self.recv_tl()?;
        let res_pq = match resp {
            TlObject::ResPQ(r) => r,
            _ => return Err(KostaError::Protocol("Expected ResPQ".into())),
        };
        if res_pq.nonce != nonce {
            return Err(KostaError::Protocol("Nonce mismatch".into()));
        }
        let server_nonce = res_pq.server_nonce;
        info!("ResPQ received, server_nonce: {:?}", server_nonce);

        // Проверяем p * q == pq
        let p_big = BigUint::from_bytes_be(&res_pq.p);
        let q_big = BigUint::from_bytes_be(&res_pq.q);
        let pq_big = BigUint::from_bytes_be(&res_pq.pq);
        if &p_big * &q_big != pq_big {
            return Err(KostaError::Protocol("p*q != pq".into()));
        }
        if !crate::dh_checks::is_prime(&p_big) || !crate::dh_checks::is_prime(&q_big) {
            return Err(KostaError::Protocol("p or q not prime".into()));
        }
        debug!("p and q verified");

        // --- проверка fingerprint ---
        let server_public = get_server_public_key();
        let expected_fingerprint = compute_key_fingerprint(&server_public);
        if !res_pq.server_public_key_fingerprints.contains(&expected_fingerprint) {
            return Err(KostaError::Protocol("Server public key fingerprint mismatch".into()));
        }
        debug!("Server public key fingerprint verified");

        // --- new_nonce ---
        let mut new_nonce_bytes = [0u8; 32];
        rng.fill(&mut new_nonce_bytes);
        let new_nonce = Int256(new_nonce_bytes);

        // --- PqInnerData ---
        let inner = PqInnerData {
            pq: res_pq.pq.clone(),
            p: res_pq.p.clone(),
            q: res_pq.q.clone(),
            nonce: nonce.clone(),
            server_nonce: server_nonce.clone(),
            new_nonce: new_nonce.clone(),
        };
        let mut inner_bytes = Vec::new();
        TlObject::PqInnerData(inner).write_boxed(&mut inner_bytes)?;

        // --- шифрование ---
        let (tmp_key, nonce_gcm) = kosta_crypto::encrypted_inner::tmp_key_from_nonce(&nonce, &server_nonce);
        let padded = kosta_crypto::padding::pad(&inner_bytes);
        let encrypted_inner = kosta_crypto::encrypted_inner::encrypt_inner(&tmp_key, &nonce_gcm, &padded)?;

        // --- ReqDHParams ---
        let req_dh = ReqDHParams {
            nonce: nonce.clone(),
            server_nonce: server_nonce.clone(),
            p: res_pq.p,
            q: res_pq.q,
            public_key_fingerprint: res_pq.server_public_key_fingerprints[0],
            encrypted_data: encrypted_inner,
        };
        info!("Sending ReqDHParams");
        self.send_tl(&TlObject::ReqDHParams(req_dh))?;

        // --- ServerDHParamsOk ---
        let resp = self.recv_tl()?;
        let dh_params = match resp {
            TlObject::ServerDHParamsOk(p) => p,
            _ => return Err(KostaError::Protocol("Expected ServerDHParamsOk".into())),
        };
        if dh_params.nonce != nonce || dh_params.server_nonce != server_nonce {
            return Err(KostaError::Protocol("DH params nonce mismatch".into()));
        }

        // Проверка подписи
        let server_public = get_server_public_key();
        let mut data_to_verify = Vec::new();
        dh_params.nonce.write_bytes(&mut data_to_verify)?;
        dh_params.server_nonce.write_bytes(&mut data_to_verify)?;
        dh_params.encrypted_answer.write_bytes(&mut data_to_verify)?;
        ServerKeypair::verify(&server_public, &data_to_verify, &dh_params.signature)
            .map_err(|e| KostaError::Protocol(format!("Signature verification failed: {}", e)))?;
        debug!("Server signature verified");

        // --- расшифровка ServerDHInnerData ---
        let (tmp_key, nonce_gcm) = kosta_crypto::encrypted_inner::tmp_key(&new_nonce, &server_nonce);
        let decrypted = kosta_crypto::encrypted_inner::decrypt_inner(&tmp_key, &nonce_gcm, &dh_params.encrypted_answer)?;
        let mut cursor = Cursor::new(&decrypted);
        let inner_obj = TlObject::read_boxed(&mut cursor)?;
        let inner_data = match inner_obj {
            TlObject::ServerDHInnerData(data) => data,
            _ => return Err(KostaError::Protocol("Expected ServerDHInnerData".into())),
        };
        if inner_data.nonce != nonce || inner_data.server_nonce != server_nonce {
            return Err(KostaError::Protocol("Inner nonce mismatch".into()));
        }
        debug!("ServerDHInnerData parsed");

        // --- проверка DH параметров ---
        let dh_prime = BigUint::from_bytes_be(&inner_data.dh_prime);
        let g = BigUint::from(inner_data.g as u32);
        if let Err(e) = crate::dh_checks::validate_dh_params(&dh_prime, &g) {
            return Err(KostaError::Protocol(e.into()));
        }
        if g <= BigUint::from(1u32) || g >= &dh_prime - BigUint::from(1u32) {
            return Err(KostaError::Protocol("g is out of range".into()));
        }
        debug!("DH parameters validated");

        // --- вычисление DH ---
        let g_a = BigUint::from_bytes_be(&inner_data.g_a);
        let b = kosta_crypto::dh::generate_private_key() % &dh_prime;
        let g_b = kosta_crypto::dh::compute_public_key(&g, &b, &dh_prime);
        let shared_secret = kosta_crypto::dh::compute_shared_secret(&g_a, &b, &dh_prime);
        debug!("Shared secret computed");

        // --- SetClientDHParams ---
        let client_inner = ClientDHInnerData {
            nonce: nonce.clone(),
            server_nonce: server_nonce.clone(),
            retry_id: 0,
            g_b: g_b.to_bytes_be(),
        };
        let mut client_inner_bytes = Vec::new();
        (CLIENT_DH_INNER_DATA_ID as i32).write_bytes(&mut client_inner_bytes)?;
        client_inner.write_bytes(&mut client_inner_bytes)?;
        let padded = kosta_crypto::padding::pad(&client_inner_bytes);
        let (tmp_key, nonce_gcm) = kosta_crypto::encrypted_inner::tmp_key(&new_nonce, &server_nonce);
        let encrypted_client = kosta_crypto::encrypted_inner::encrypt_inner(&tmp_key, &nonce_gcm, &padded)?;

        let set_dh = SetClientDHParams {
            nonce: nonce.clone(),
            server_nonce: server_nonce.clone(),
            encrypted_data: encrypted_client,
        };
        info!("Sending SetClientDHParams");
        self.send_tl(&TlObject::SetClientDHParams(set_dh))?;

        // --- DHGenOk ---
        let resp = self.recv_tl()?;
        let dh_gen = match resp {
            TlObject::DHGenOk(gen) => gen,
            _ => return Err(KostaError::Protocol("Expected DHGenOk".into())),
        };
        if dh_gen.nonce != nonce || dh_gen.server_nonce != server_nonce {
            return Err(KostaError::Protocol("DHGen nonce mismatch".into()));
        }
        debug!("DHGenOk verified");

        // --- проверка new_nonce_hash1 ---
        let mut hasher = Sha1::new();
        hasher.update(&shared_secret.to_bytes_le());
        let hash = hasher.finalize();
        let mut expected_hash = [0u8; 16];
        expected_hash.copy_from_slice(&hash[0..16]);
        if expected_hash != dh_gen.new_nonce_hash1.0 {
            return Err(KostaError::Protocol("new_nonce_hash1 mismatch".into()));
        }
        debug!("new_nonce_hash1 verified");

        // --- создание сессии ---
        let client_nonce_arr = nonce.0;
        let server_nonce_arr = server_nonce.0;
        let crypto_ctx = SessionCrypto::new(
            &shared_secret.to_bytes_le(),
            &client_nonce_arr,
            &server_nonce_arr,
        );
        let auth_key = AuthKey::from_shared_secret(&shared_secret, &client_nonce_arr, &server_nonce_arr);
        let auth_key_id = compute_auth_key_id(&shared_secret);
        self.auth_key_id = Some(auth_key_id);
        self.crypto = Some(crypto_ctx);
        self.auth_key = Some(auth_key);
        self.server_salt = i64::from_le_bytes(
            server_nonce.0[0..8].try_into()
                .map_err(|_| KostaError::Protocol("Invalid server_nonce length".into()))?
        );
        self.recv_last_msg_id = 0;
        info!("Authentication complete, auth_key_id: {}", auth_key_id);

        Ok(())
    }

    /// Отправляет TL-объект (автоматически шифрует, если сессия аутентифицирована).
    pub fn send_tl(&mut self, obj: &TlObject) -> Result<(), KostaError> {
        let mut plain_bytes = Vec::new();
        if self.crypto.is_some() {
            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
            let time_part = (now.as_millis() as i64) << 32;
            let msg_id = time_part | (self.msg_id_counter as i64);
            self.msg_id_counter = self.msg_id_counter.wrapping_add(1);
            self.last_msg_id = msg_id;
            self.seq_no += 1;
            msg_id.write_bytes(&mut plain_bytes)?;
            self.seq_no.write_bytes(&mut plain_bytes)?;
        }
        obj.write_boxed(&mut plain_bytes)?;
        debug!("send_tl: serialized {} bytes", plain_bytes.len());

        if let Some(crypto) = &mut self.crypto {
            let (nonce, ciphertext, tag) = crypto.encrypt_outgoing(&plain_bytes)?;
            let auth_key_id = self.auth_key_id.unwrap_or(0);
            let mut msg = Vec::with_capacity(8 + 16 + ciphertext.len() + 16);
            msg.extend_from_slice(&auth_key_id.to_le_bytes());
            msg.extend_from_slice(&nonce);
            msg.extend_from_slice(&ciphertext);
            msg.extend_from_slice(&tag);
            debug!("send_tl: encrypted message, total size: {} bytes", msg.len());
            self.transport.send(&msg)?;
        } else {
            debug!("send_tl: sending plaintext (unencrypted)");
            self.transport.send(&plain_bytes)?;
        }
        Ok(())
    }

    /// Получает TL-объект (автоматически расшифровывает, если сессия аутентифицирована).
    pub fn recv_tl(&mut self) -> Result<TlObject, KostaError> {
        debug!("recv_tl: calling transport.recv()...");
        let raw = self.transport.recv()?;
        debug!("recv_tl: got {} bytes", raw.len());

        if let Some(crypto) = &mut self.crypto {
            if raw.len() < 8 + 16 + 16 {
                return Err(KostaError::Protocol("Message too short".into()));
            }
            let auth_key_id = i64::from_le_bytes(
                raw[0..8].try_into()
                    .map_err(|_| KostaError::Protocol("Invalid auth_key_id length".into()))?
            );
            if auth_key_id != self.auth_key_id.unwrap_or(0) {
                return Err(KostaError::Protocol("AuthKey ID mismatch".into()));
            }
            let nonce: [u8; 16] = raw[8..24].try_into()
                .map_err(|_| KostaError::Protocol("Invalid nonce length".into()))?;
            let tag_start = raw.len() - 16;
            let tag: [u8; 16] = raw[tag_start..].try_into()
                .map_err(|_| KostaError::Protocol("Invalid tag length".into()))?;
            let ciphertext = &raw[24..tag_start];
            debug!("recv_tl: decrypting (auth_key_id={})", auth_key_id);

            let plaintext = crypto.decrypt_incoming(&nonce, ciphertext, &tag)?;
            debug!("recv_tl: decrypted {} bytes", plaintext.len());
            let mut cursor = Cursor::new(&plaintext);

            let msg_id = i64::read_bytes(&mut cursor)?;
            let seq_no = i32::read_bytes(&mut cursor)?;
            debug!("recv_tl: msg_id={}, seq_no={}", msg_id, seq_no);

            if msg_id <= self.recv_last_msg_id {
                return Err(KostaError::Protocol("msg_id not monotonically increasing".into()));
            }
            self.recv_last_msg_id = msg_id;

            if seq_no != self.recv_seq_no + 1 {
                return Err(KostaError::Protocol("Unexpected seq_no (possible replay)".into()));
            }
            self.recv_seq_no = seq_no;

            let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
            let now_msg_id = (now.as_millis() as i64) << 32;
            let diff_ms = ((msg_id - now_msg_id) >> 32).abs();
            if diff_ms > 30_000 {
                return Err(KostaError::Protocol("Message timestamp out of range".into()));
            }

            let obj = TlObject::read_boxed(&mut cursor)?;
            debug!("recv_tl: parsed TlObject: {:?}", obj);
            Ok(obj)
        } else {
            debug!("recv_tl: plaintext (unencrypted)");
            let mut cursor = Cursor::new(&raw);
            let obj = TlObject::read_boxed(&mut cursor)?;
            debug!("recv_tl: parsed TlObject: {:?}", obj);
            Ok(obj)
        }
    }

    // -------------------------------------------------------------------------
    // Высокоуровневые методы
    // -------------------------------------------------------------------------

    /// Отправляет сообщение в чат.
    pub fn send_message(&mut self, chat_id: ChatId, text: String) -> Result<Message, KostaError> {
        let random_id = OsRng.gen::<i64>();
        let req = TlObject::SendMessage(SendMessage {
            chat_id,
            text,
            random_id,
        });
        self.send_tl(&req)?;
        let resp = self.recv_tl()?;
        match resp {
            TlObject::Message(msg) => Ok(msg),
            _ => Err(KostaError::Protocol("Expected Message response".into())),
        }
    }

    /// Запрашивает историю сообщений.
    pub fn get_history(&mut self, chat_id: ChatId, offset: i32, limit: i32) -> Result<HistoryResult, KostaError> {
        let req = TlObject::GetHistory(GetHistory { chat_id, offset, limit });
        self.send_tl(&req)?;
        let resp = self.recv_tl()?;
        match resp {
            TlObject::HistoryResult(result) => Ok(result),
            _ => Err(KostaError::Protocol("Expected HistoryResult".into())),
        }
    }

    /// Отправляет подтверждение получения сообщения.
    pub fn send_ack(&mut self, message_id: MessageId) -> Result<(), KostaError> {
        let req = TlObject::SendMessageAck(SendMessageAck { message_id });
        self.send_tl(&req)?;
        Ok(())
    }

    /// Обновляет статус текущего пользователя.
    pub fn update_status(&mut self, status: UserStatus) -> Result<(), KostaError> {
        let user_id = self.user_id.clone().unwrap_or(UserId(0));
        let req = TlObject::UserStatusUpdate(UserStatusUpdate { user_id, status });
        self.send_tl(&req)?;
        Ok(())
    }
}