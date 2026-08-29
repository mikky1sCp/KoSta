// kosta/src/session.rs
use kosta_core::tl::types::{Int128, Int256, TlWrite, TlRead};
use kosta_core::tl::constructors::*;
use kosta_crypto::session_crypto::SessionCrypto;
use kosta_crypto::AuthKey;
use kosta_transport::{Transport, StreamTransport, tls, MockTransport};
use crate::error::KostaError;
use rand::Rng;
use std::net::TcpStream;
use std::io::Cursor;
use num_bigint::BigUint;
use sha1::{Sha1, Digest};
use crate::server_keys::{ServerKeypair, get_server_public_key};
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::info;

// --- вспомогательная функция для fingerprint ---
fn compute_key_fingerprint(public_key: &ed25519_dalek::VerifyingKey) -> i64 {
    let mut hasher = sha1::Sha1::new();
    hasher.update(public_key.as_bytes());
    let hash = hasher.finalize();
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&hash[12..20]);
    i64::from_le_bytes(buf)
}

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
}

impl Session<Box<dyn Transport>> {
    pub fn connect(host: &str, port: u16, use_tls: bool, tls_domain: &str) -> Result<Self, KostaError> {
        let transport: Box<dyn Transport> = if use_tls {
            let tls_stream = tls::connect_tls(host, port, tls_domain)
                .map_err(|e| KostaError::ConnectionFailed(format!("TLS connection failed: {}", e)))?;
            Box::new(tls_stream)
        } else {
            let stream = TcpStream::connect((host, port))?;
            Box::new(StreamTransport::new(stream))
        };
        let mut rng = rand::thread_rng();
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
        })
    }

    pub fn from_transport(transport: StreamTransport<TcpStream>) -> Self {
        let mut rng = rand::thread_rng();
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
        }
    }

    pub fn reconnect(&mut self) -> Result<(), KostaError> {
        if let (Some(host), Some(port)) = (self.host.as_ref(), self.port) {
            let use_tls = self.use_tls;
            let domain = self.tls_domain.as_deref().unwrap_or(host);
            let transport: Box<dyn Transport> = if use_tls {
                let tls_stream = tls::connect_tls(host, port, domain)
                    .map_err(|e| KostaError::ConnectionFailed(format!("TLS reconnect failed: {}", e)))?;
                Box::new(tls_stream)
            } else {
                let stream = TcpStream::connect((host.as_str(), port))?; // <-- исправлено: убрана *
                Box::new(StreamTransport::new(stream))
            };
            self.transport = transport;
            Ok(())
        } else {
            Err(KostaError::ConnectionFailed("No host/port stored for reconnect".into()))
        }
    }
}

impl Session<MockTransport> {
    pub fn from_mock_transport(transport: MockTransport) -> Self {
        let mut rng = rand::thread_rng();
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
        }
    }
}

impl<T: Transport> Session<T> {
    pub fn authenticate(&mut self) -> Result<(), KostaError> {
        let mut rng = rand::thread_rng();

        // --- шаг 1: req_pq ---
        let mut nonce_bytes = [0u8; 16];
        rng.fill(&mut nonce_bytes);
        let nonce = Int128(nonce_bytes);
        info!("Sending ReqPq with nonce: {:?}", nonce);
        self.send_tl(&TlObject::ReqPq(ReqPq { nonce: nonce.clone() }))?;
        info!("Sent ReqPq, waiting for ResPQ...");

        // --- шаг 2: ResPQ ---
        let resp = self.recv_tl()?;
        info!("Received response after ReqPq");
        let res_pq = match resp {
            TlObject::ResPQ(r) => r,
            _ => return Err(KostaError::Protocol("Expected ResPQ".into())),
        };
        if res_pq.nonce != nonce {
            return Err(KostaError::Protocol("Nonce mismatch".into()));
        }
        let server_nonce = res_pq.server_nonce;
        info!("ResPQ received, server_nonce: {:?}", server_nonce);

        // --- проверка fingerprint серверного ключа ---
        let server_public = get_server_public_key();
        let expected_fingerprint = compute_key_fingerprint(&server_public);
        if !res_pq.server_public_key_fingerprints.contains(&expected_fingerprint) {
            return Err(KostaError::Protocol("Server public key fingerprint mismatch".into()));
        }
        info!("Server public key fingerprint verified");

        // --- шаг 3: факторизация pq ---
        let pq_num = BigUint::from_bytes_be(&res_pq.pq);
        info!("Factoring pq ({} bytes)", res_pq.pq.len());
        let (p, q) = crate::factor::factor_pq(&pq_num)?;
        info!("Factors found: p={:?}, q={:?}", p, q);

        // --- шаг 4: new_nonce ---
        let mut new_nonce_bytes = [0u8; 32];
        rng.fill(&mut new_nonce_bytes);
        let new_nonce = Int256(new_nonce_bytes);
        info!("Generated new_nonce");

        // --- шаг 5: PqInnerData ---
        let inner = PqInnerData {
            pq: res_pq.pq,
            p: p.clone(),
            q: q.clone(),
            nonce: nonce.clone(),
            server_nonce: server_nonce.clone(),
            new_nonce: new_nonce.clone(),
        };
        let mut inner_bytes = Vec::new();
        TlObject::PqInnerData(inner).write_boxed(&mut inner_bytes)?;
        info!("PqInnerData serialized, size: {} bytes", inner_bytes.len());

        // --- шаг 6: шифрование inner данных (GCM) - используем nonce, а не new_nonce ---
        let (tmp_key, nonce_gcm) = kosta_crypto::encrypted_inner::tmp_key_from_nonce(&nonce, &server_nonce);
        let padded = kosta_crypto::padding::pad(&inner_bytes);
        let encrypted_inner = kosta_crypto::encrypted_inner::encrypt_inner(&tmp_key, &nonce_gcm, &padded)?;
        info!("Encrypted inner data, size: {} bytes", encrypted_inner.len());

        // --- шаг 7: ReqDHParams ---
        let req_dh = ReqDHParams {
            nonce: nonce.clone(),
            server_nonce: server_nonce.clone(),
            p,
            q,
            public_key_fingerprint: res_pq.server_public_key_fingerprints[0],
            encrypted_data: encrypted_inner,
        };
        info!("Sending ReqDHParams");
        self.send_tl(&TlObject::ReqDHParams(req_dh))?;
        info!("Sent ReqDHParams, waiting for ServerDHParamsOk...");

        // --- шаг 8: ServerDHParamsOk (с подписью) ---
        let resp = self.recv_tl()?;
        info!("Received ServerDHParamsOk");
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
        info!("Server signature verified");

        // --- шаг 9: расшифровка ServerDHInnerData (GCM) - используем new_nonce ---
        let (tmp_key, nonce_gcm) = kosta_crypto::encrypted_inner::tmp_key(&new_nonce, &server_nonce);
        let decrypted = kosta_crypto::encrypted_inner::decrypt_inner(&tmp_key, &nonce_gcm, &dh_params.encrypted_answer)?;
        info!("Decrypted ServerDHInnerData, size: {} bytes", decrypted.len());
        let mut cursor = Cursor::new(&decrypted);
        let inner_obj = TlObject::read_boxed(&mut cursor)?;
        let inner_data = match inner_obj {
            TlObject::ServerDHInnerData(data) => data,
            _ => return Err(KostaError::Protocol("Expected ServerDHInnerData".into())),
        };
        if inner_data.nonce != nonce || inner_data.server_nonce != server_nonce {
            return Err(KostaError::Protocol("Inner nonce mismatch".into()));
        }
        info!("ServerDHInnerData parsed");

        // --- шаг 10: проверка параметров DH ---
        let dh_prime = BigUint::from_bytes_be(&inner_data.dh_prime);
        let g = BigUint::from(inner_data.g as u32);
        if let Err(e) = crate::dh_checks::validate_dh_params(&dh_prime, &g) {
            return Err(KostaError::Protocol(e.into()));
        }
        if g <= BigUint::from(1u32) || g >= &dh_prime - BigUint::from(1u32) {
            return Err(KostaError::Protocol("g is out of range".into()));
        }
        info!("DH parameters validated");

        // --- шаг 11: вычисление DH ---
        let g_a = BigUint::from_bytes_be(&inner_data.g_a);
        let b = kosta_crypto::dh::generate_private_key() % &dh_prime;
        let g_b = kosta_crypto::dh::compute_public_key(&g, &b, &dh_prime);
        let shared_secret = kosta_crypto::dh::compute_shared_secret(&g_a, &b, &dh_prime);
        info!("Shared secret computed");

        // --- шаг 12: SetClientDHParams - шифруем с ключом от new_nonce ---
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
        info!("Encrypted SetClientDHParams, size: {} bytes", encrypted_client.len());

        let set_dh = SetClientDHParams {
            nonce: nonce.clone(),
            server_nonce: server_nonce.clone(),
            encrypted_data: encrypted_client,
        };
        info!("Sending SetClientDHParams");
        self.send_tl(&TlObject::SetClientDHParams(set_dh))?;
        info!("Sent SetClientDHParams, waiting for DHGenOk...");

        // --- шаг 13: DHGenOk ---
        let resp = self.recv_tl()?;
        info!("Received DHGenOk");
        let dh_gen = match resp {
            TlObject::DHGenOk(gen) => gen,
            _ => return Err(KostaError::Protocol("Expected DHGenOk".into())),
        };
        if dh_gen.nonce != nonce || dh_gen.server_nonce != server_nonce {
            return Err(KostaError::Protocol("DHGen nonce mismatch".into()));
        }
        info!("DHGenOk verified");

        // --- ПРОВЕРКА new_nonce_hash1 ---
        let mut hasher = Sha1::new();
        hasher.update(&shared_secret.to_bytes_le());
        let hash = hasher.finalize();
        let mut expected_hash = [0u8; 16];
        expected_hash.copy_from_slice(&hash[0..16]);
        if expected_hash != dh_gen.new_nonce_hash1.0 {
            return Err(KostaError::Protocol("new_nonce_hash1 mismatch".into()));
        }
        info!("new_nonce_hash1 verified");

        // --- шаг 14: создание SessionCrypto и сохранение auth_key ---
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
        self.server_salt = i64::from_le_bytes(server_nonce.0[0..8].try_into().map_err(|_| KostaError::Protocol("Invalid server_nonce length".into()))?);
        self.recv_last_msg_id = 0;
        info!("Authentication complete, auth_key_id: {}", auth_key_id);

        Ok(())
    }

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
        info!("send_tl: serialized {} bytes", plain_bytes.len());

        if let Some(crypto) = &mut self.crypto {
            let (nonce, ciphertext, tag) = crypto.encrypt_outgoing(&plain_bytes)?;
            let auth_key_id = self.auth_key_id.unwrap_or(0);
            let mut msg = Vec::with_capacity(8 + 16 + ciphertext.len() + 16);
            msg.extend_from_slice(&auth_key_id.to_le_bytes());
            msg.extend_from_slice(&nonce);
            msg.extend_from_slice(&ciphertext);
            msg.extend_from_slice(&tag);
            info!("send_tl: encrypted message, total size: {} bytes", msg.len());
            self.transport.send(&msg)?;
        } else {
            info!("send_tl: sending plaintext (unencrypted)");
            self.transport.send(&plain_bytes)?;
        }
        Ok(())
    }

    pub fn recv_tl(&mut self) -> Result<TlObject, KostaError> {
        info!("recv_tl: calling transport.recv()...");
        let raw = self.transport.recv()?;
        info!("recv_tl: got {} bytes", raw.len());

        if let Some(crypto) = &mut self.crypto {
            if raw.len() < 8 + 16 + 16 {
                return Err(KostaError::Protocol("Message too short".into()));
            }
            let auth_key_id = i64::from_le_bytes(raw[0..8].try_into().unwrap());
            if auth_key_id != self.auth_key_id.unwrap_or(0) {
                return Err(KostaError::Protocol("AuthKey ID mismatch".into()));
            }
            let mut nonce = [0u8; 16];
            nonce.copy_from_slice(&raw[8..24]);
            let tag_start = raw.len() - 16;
            let mut tag = [0u8; 16];
            tag.copy_from_slice(&raw[tag_start..]);
            let ciphertext = &raw[24..tag_start];
            info!("recv_tl: decrypting (auth_key_id={}, nonce={:?})", auth_key_id, nonce);

            let plaintext = crypto.decrypt_incoming(&nonce, ciphertext, &tag)?;
            info!("recv_tl: decrypted {} bytes", plaintext.len());
            let mut cursor = Cursor::new(&plaintext);

            let msg_id = i64::read_bytes(&mut cursor)?;
            let seq_no = i32::read_bytes(&mut cursor)?;
            info!("recv_tl: msg_id={}, seq_no={}", msg_id, seq_no);

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
            info!("recv_tl: parsed TlObject: {:?}", obj);
            Ok(obj)
        } else {
            info!("recv_tl: plaintext (unencrypted)");
            let mut cursor = Cursor::new(&raw);
            let obj = TlObject::read_boxed(&mut cursor)?;
            info!("recv_tl: parsed TlObject: {:?}", obj);
            Ok(obj)
        }
    }

    pub fn send_message(&mut self, chat_id: ChatId, text: String) -> Result<Message, KostaError> {
        let random_id = rand::thread_rng().gen::<i64>();
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

    pub fn get_history(&mut self, chat_id: ChatId, offset: i32, limit: i32) -> Result<HistoryResult, KostaError> {
        let req = TlObject::GetHistory(GetHistory { chat_id, offset, limit });
        self.send_tl(&req)?;
        let resp = self.recv_tl()?;
        match resp {
            TlObject::HistoryResult(result) => Ok(result),
            _ => Err(KostaError::Protocol("Expected HistoryResult".into())),
        }
    }

    pub fn send_ack(&mut self, message_id: MessageId) -> Result<(), KostaError> {
        let req = TlObject::SendMessageAck(SendMessageAck { message_id });
        self.send_tl(&req)?;
        Ok(())
    }

    pub fn update_status(&mut self, status: UserStatus) -> Result<(), KostaError> {
        let user_id = self.user_id.clone().unwrap_or(UserId(0));
        let req = TlObject::UserStatusUpdate(UserStatusUpdate { user_id, status });
        self.send_tl(&req)?;
        Ok(())
    }
}

fn compute_auth_key_id(shared_secret: &BigUint) -> i64 {
    let secret_bytes = shared_secret.to_bytes_le();
    let mut hasher = Sha1::new();
    hasher.update(&secret_bytes);
    let hash = hasher.finalize();
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&hash[12..20]);
    i64::from_le_bytes(bytes)
}