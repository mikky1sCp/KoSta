//! Клиент MTProto 2.0: хендшейк, RPC-вызовы, подтверждения и повторы.
//!
//! Это «новый» клиентский слой рядом с легаси [`crate::Session`]: он говорит
//! на MTProto 2.0 (`AES-256-IGE` + `msg_key`, контейнеры, `rpc_result`).
//! После cutover (этап 6) старый слой будет удалён, а этот станет основным.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use num_bigint::BigUint;
use rand::rngs::OsRng;
use rand::RngCore;

use kosta_crypto::dh::{
    compute_public_key, compute_shared_secret, generate_private_key_in_range, shared_secret_bytes,
};
use kosta_crypto::dh_checks::{
    factor_semiprime, is_prime, validate_dh_params, validate_public_value,
};
use kosta_crypto::encrypted_inner::{
    decrypt_inner, encrypt_inner, tmp_key_bytes, tmp_key_from_nonce_bytes,
};
use kosta_crypto::handshake::new_nonce_hash1;
use kosta_crypto::mtproto2::{self, AuthKey, Direction};
use kosta_crypto::padding;
use kosta_protocol::ack::AckTracker;
use kosta_protocol::frame;
use kosta_protocol::msg_id::{now_secs, MsgIdGenerator, MsgIdValidator, SeqNoGenerator};
use kosta_protocol::ProtocolError;
use kosta_tl::{
    Bytes, ChatMessage, ClientDhInnerData, CreateGroup, CreatePrivateDialog, DialogInfo,
    DialogList, GetDialogs, GetHistory, HistoryResult, Int128, Int256, Message, MsgContainer,
    MsgsAck, PqInnerData, Reader, ReqDhParams, ReqPq, ServerDhInnerData, ServerDhParams,
    SetClientDhParams, SetClientDhParamsAnswer, SignUp, TlObject, TlRead, TlWrite,
    UpdateUserStatus, UserStatus,
};
use kosta_transport::mtproto_abridged::{decode_frame, encode_frame, write_init};
use kosta_transport::tls::{connect_tls_stream, ClientTlsStream};
use kosta_transport::TransportError;

use crate::error::KostaError;
use crate::server_keys::get_server_public_key;

/// Параметры подключения клиента.
#[derive(Debug, Clone, Default)]
pub struct ConnectOptions {
    /// Использовать TLS.
    pub use_tls: bool,
    /// Домен для проверки сертификата; по умолчанию берётся `host`.
    pub tls_domain: Option<String>,
    /// ⚠️ Только для локальной отладки с самоподписанным сертификатом:
    /// отключает проверку сертификата и имени хоста (TLS перестаёт защищать от MITM).
    pub allow_invalid_certs: bool,
    /// Таймаут операций в секундах; 0 — значение по умолчанию.
    pub timeout_secs: u64,
}

/// Соединение клиента: TCP или TLS.
#[allow(clippy::large_enum_variant)]
enum Conn {
    Plain(TcpStream),
    Tls(ClientTlsStream),
}

impl Read for Conn {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(stream) => stream.read(buffer),
            Conn::Tls(stream) => stream.read(buffer),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Plain(stream) => stream.write(buffer),
            Conn::Tls(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Plain(stream) => stream.flush(),
            Conn::Tls(stream) => stream.flush(),
        }
    }
}

/// Клиентская сессия MTProto 2.0.
pub struct MtprotoSession {
    conn: Conn,
    server_public: VerifyingKey,
    auth_key: AuthKey,
    auth_key_id: i64,
    session_id: i64,
    salt: i64,
    user_id: Option<i64>,
    plain_counter: u32,
    msg_ids: MsgIdGenerator,
    seq: SeqNoGenerator,
    server_ids: MsgIdValidator,
    acks: AckTracker,
    timeout: Duration,
    authenticated: bool,
}

impl MtprotoSession {
    /// Подключается к серверу (TCP или TLS) и отправляет init-байт abridged.
    pub fn connect(host: &str, port: u16, opts: ConnectOptions) -> Result<Self, KostaError> {
        let timeout = if opts.timeout_secs > 0 {
            Duration::from_secs(opts.timeout_secs)
        } else {
            Duration::from_secs(30)
        };

        let conn = if opts.use_tls {
            let domain = opts.tls_domain.clone().unwrap_or_else(|| host.to_string());
            Conn::Tls(connect_tls_stream(
                host,
                port,
                &domain,
                opts.allow_invalid_certs,
                Some(timeout),
            )?)
        } else {
            let stream = TcpStream::connect((host, port))?;
            stream.set_read_timeout(Some(timeout))?;
            stream.set_write_timeout(Some(timeout))?;
            Conn::Plain(stream)
        };

        let mut session = MtprotoSession {
            conn,
            server_public: get_server_public_key()?,
            auth_key: [0u8; 256],
            auth_key_id: 0,
            session_id: 0,
            salt: 0,
            user_id: None,
            plain_counter: 0,
            msg_ids: MsgIdGenerator::client(),
            seq: SeqNoGenerator::new(),
            server_ids: MsgIdValidator::server(),
            acks: AckTracker::new(),
            timeout,
            authenticated: false,
        };
        write_init(&mut session.conn)
            .map_err(|e| KostaError::ConnectionFailed(format!("abridged init failed: {e}")))?;
        Ok(session)
    }

    /// `auth_key_id` текущей сессии (0 до аутентификации).
    pub fn auth_key_id(&self) -> i64 {
        self.auth_key_id
    }

    /// Идентификатор пользователя после `sign_up`.
    pub fn user_id(&self) -> Option<i64> {
        self.user_id
    }

    /// Текущий серверный salt.
    pub fn salt(&self) -> i64 {
        self.salt
    }

    /// Прошла ли сессия хендшейк.
    pub fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    /// Таймаут операций.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    fn send_frame(&mut self, payload: &[u8]) -> Result<(), KostaError> {
        let frame = encode_frame(payload)?;
        self.conn.write_all(&frame)?;
        self.conn.flush()?;
        Ok(())
    }

    fn read_frame(&mut self) -> Result<Vec<u8>, KostaError> {
        Ok(decode_frame(&mut self.conn)?)
    }

    fn send_plain(&mut self, object: &TlObject) -> Result<(), KostaError> {
        let body = serialize(object)?;
        self.plain_counter = self.plain_counter.wrapping_add(1);
        let msg_id = (now_secs() << 32) | (((self.plain_counter & 0x3FFF_FFFF) as i64) << 2);
        let frame = frame::build_plain_frame(msg_id, &body)?;
        self.send_frame(&frame)
    }

    fn read_plain(&mut self) -> Result<TlObject, KostaError> {
        let raw = self.read_frame()?;
        let plain = frame::parse_plain_frame(&raw)?;
        Ok(TlObject::read_boxed(&mut Reader::new(&plain.body))?)
    }

    /// Полный цикл `req_pq` → `dh_gen_ok`. Идемпотентен.
    pub fn authenticate(&mut self) -> Result<(), KostaError> {
        if self.authenticated {
            return Ok(());
        }

        let nonce = random_int128();
        self.send_plain(&TlObject::ReqPq(ReqPq {
            nonce: Int128(nonce),
        }))?;

        let res_pq = match self.read_plain()? {
            TlObject::ResPq(res_pq) => res_pq,
            other => return Err(unexpected("resPQ", &other)),
        };
        if res_pq.nonce.0 != nonce {
            return Err(protocol("resPQ nonce mismatch"));
        }
        let fingerprint = key_fingerprint(&self.server_public);
        if !res_pq.server_public_key_fingerprints.contains(&fingerprint) {
            return Err(protocol("server public key fingerprint mismatch"));
        }

        let pq =
            bytes_to_u64(&res_pq.pq.0).ok_or_else(|| protocol("resPQ.pq must fit into u64"))?;
        let (p, q) = factor_semiprime(pq).ok_or_else(|| protocol("failed to factor resPQ.pq"))?;
        if !is_prime(&BigUint::from(p)) || !is_prime(&BigUint::from(q)) {
            return Err(protocol("pq factors are not prime"));
        }

        let new_nonce = random_int256();
        let inner = PqInnerData {
            pq: Bytes(res_pq.pq.0.clone()),
            p: Bytes(BigUint::from(p).to_bytes_be()),
            q: Bytes(BigUint::from(q).to_bytes_be()),
            nonce: Int128(nonce),
            server_nonce: res_pq.server_nonce,
            new_nonce: Int256(new_nonce),
        };
        let padded = padding::pad(&serialize(&inner)?);
        let (tmp_aes, tmp_nonce) = tmp_key_from_nonce_bytes(&nonce, &res_pq.server_nonce.0);
        let encrypted_inner = encrypt_inner(&tmp_aes, &tmp_nonce, &padded)?;

        self.send_plain(&TlObject::ReqDhParams(ReqDhParams {
            nonce: Int128(nonce),
            server_nonce: res_pq.server_nonce,
            p: Bytes(BigUint::from(p).to_bytes_be()),
            q: Bytes(BigUint::from(q).to_bytes_be()),
            public_key_fingerprint: fingerprint,
            encrypted_data: Bytes(encrypted_inner),
        }))?;

        let (dh_nonce, dh_server_nonce, encrypted_answer, signature) = match self.read_plain()? {
            TlObject::ServerDhParams(ServerDhParams::ServerDhParamsOk {
                nonce,
                server_nonce,
                encrypted_answer,
                signature,
            }) => (nonce, server_nonce, encrypted_answer, signature),
            other => return Err(unexpected("server_DH_params_ok", &other)),
        };
        if dh_nonce.0 != nonce || dh_server_nonce.0 != res_pq.server_nonce.0 {
            return Err(protocol("server_DH_params_ok nonce mismatch"));
        }

        let mut signed = Vec::with_capacity(16 + 16 + encrypted_answer.0.len());
        signed.extend_from_slice(&nonce);
        signed.extend_from_slice(&res_pq.server_nonce.0);
        signed.extend_from_slice(&encrypted_answer.0);
        let signature_bytes: [u8; 64] = signature
            .0
            .as_slice()
            .try_into()
            .map_err(|_| protocol("server signature must be 64 bytes"))?;
        self.server_public
            .verify(&signed, &Signature::from_bytes(&signature_bytes))
            .map_err(|e| protocol(format!("server signature verification failed: {e}")))?;

        let (aes_key, aes_nonce) = tmp_key_bytes(&new_nonce, &res_pq.server_nonce.0);
        let decrypted = decrypt_inner(&aes_key, &aes_nonce, &encrypted_answer.0)?;
        let server_inner = ServerDhInnerData::read_bytes(&mut Reader::new(&decrypted))?;
        if server_inner.nonce.0 != nonce || server_inner.server_nonce.0 != res_pq.server_nonce.0 {
            return Err(protocol("server_DH_inner_data nonce mismatch"));
        }

        let dh_prime = BigUint::from_bytes_be(&server_inner.dh_prime.0);
        let g = BigUint::from(server_inner.g as u32);
        validate_dh_params(&dh_prime, &g).map_err(|e| protocol(format!("bad DH params: {e}")))?;
        let g_a = BigUint::from_bytes_be(&server_inner.g_a.0);
        validate_public_value(&g_a, &dh_prime).map_err(|e| protocol(format!("bad g_a: {e}")))?;

        let dh_private = generate_private_key_in_range(&dh_prime);
        let g_b = compute_public_key(&g, &dh_private, &dh_prime);
        let shared = compute_shared_secret(&g_a, &dh_private, &dh_prime);
        if shared <= BigUint::from(1u32) {
            return Err(protocol("degenerate shared secret"));
        }

        let client_inner = ClientDhInnerData {
            nonce: Int128(nonce),
            server_nonce: res_pq.server_nonce,
            retry_id: 0,
            g_b: Bytes(g_b.to_bytes_be()),
        };
        let padded = padding::pad(&serialize(&client_inner)?);
        let (aes_key, aes_nonce) = tmp_key_bytes(&new_nonce, &res_pq.server_nonce.0);
        let encrypted_client = encrypt_inner(&aes_key, &aes_nonce, &padded)?;

        self.send_plain(&TlObject::SetClientDhParams(SetClientDhParams {
            nonce: Int128(nonce),
            server_nonce: res_pq.server_nonce,
            encrypted_data: Bytes(encrypted_client),
        }))?;

        let returned_hash = match self.read_plain()? {
            TlObject::SetClientDhParamsAnswer(SetClientDhParamsAnswer::DhGenOk {
                new_nonce_hash1,
                ..
            }) => new_nonce_hash1,
            other => return Err(unexpected("dh_gen_ok", &other)),
        };
        if returned_hash.0 != new_nonce_hash1(&new_nonce, &res_pq.server_nonce.0) {
            return Err(protocol("new_nonce_hash1 mismatch"));
        }

        self.auth_key = shared_secret_bytes(&shared);
        self.auth_key_id = mtproto2::auth_key_id(&self.auth_key);
        self.session_id = random_non_zero_i64();
        self.salt = 0;
        self.server_ids = MsgIdValidator::server();
        self.authenticated = true;
        Ok(())
    }

    /// Вызывает RPC-метод и возвращает результат из `rpc_result`.
    pub fn invoke(&mut self, request: TlObject) -> Result<TlObject, KostaError> {
        if !self.authenticated {
            return Err(protocol("session is not authenticated"));
        }

        let request_msg_id = self.msg_ids.next(now_secs());
        let request_seq_no = self.seq.next_content();
        let request_body = serialize(&request)?;
        let payload = self.build_request_payload(request_msg_id, request_seq_no, request_body)?;
        self.send_frame(&payload)?;
        self.wait_for_rpc(request_msg_id, &payload)
    }

    fn build_request_payload(
        &mut self,
        request_msg_id: i64,
        request_seq_no: i32,
        request_body: Vec<u8>,
    ) -> Result<Vec<u8>, KostaError> {
        let acks: Vec<i64> = self.acks.drain();
        let has_acks = !acks.is_empty();
        let body = if has_acks {
            let ack_body = serialize(&TlObject::MsgsAck(MsgsAck { msg_ids: acks }))?;
            let mut messages = Vec::with_capacity(2);
            messages.push(Message {
                msg_id: request_msg_id,
                seqno: request_seq_no,
                bytes: request_body.len() as i32,
                body: Bytes(request_body),
            });
            let ack_msg_id = self.msg_ids.next(now_secs());
            messages.push(Message {
                msg_id: ack_msg_id,
                seqno: 0,
                bytes: ack_body.len() as i32,
                body: Bytes(ack_body),
            });
            serialize(&TlObject::MsgContainer(MsgContainer { messages }))?
        } else {
            request_body
        };

        let outer_msg_id = self.msg_ids.next(now_secs());
        let (inner_msg_id, seq_no) = if has_acks {
            (outer_msg_id, 0)
        } else {
            (request_msg_id, request_seq_no)
        };
        Ok(frame::seal_frame(
            &self.auth_key,
            Direction::ClientToServer,
            self.auth_key_id,
            self.session_id,
            self.salt,
            inner_msg_id,
            seq_no,
            &body,
        )?)
    }

    fn wait_for_rpc(
        &mut self,
        request_msg_id: i64,
        retry_payload: &[u8],
    ) -> Result<TlObject, KostaError> {
        let mut retried = false;
        loop {
            match self.process_next_frame(request_msg_id) {
                Ok(Some(result)) => return Ok(result),
                Ok(None) => {}
                Err(error) if is_timeout(&error) && !retried => {
                    retried = true;
                    // Повтор с тем же msg_id: сервер вернёт сохранённый ответ.
                    self.send_frame(retry_payload)?;
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn process_next_frame(&mut self, wanted: i64) -> Result<Option<TlObject>, KostaError> {
        let raw = self.read_frame()?;
        let opened = frame::open_frame(
            &self.auth_key,
            Direction::ServerToClient,
            self.auth_key_id,
            &raw,
        )?;
        if self.salt != 0 && opened.salt != self.salt {
            return Err(protocol(format!(
                "unexpected salt {} (have {})",
                opened.salt, self.salt
            )));
        }

        let payload = opened.payload.clone();
        let first_id = Reader::new(&payload).read_u32()?;
        if first_id == kosta_tl::MSG_CONTAINER_ID {
            let mut reader = Reader::new(&payload);
            let container = MsgContainer::read_bytes(&mut reader)?;
            let consumed = reader.position();
            mtproto2::check_padding(payload.len(), consumed)?;
            let messages = container.messages.clone();

            let mut found = None;
            for message in &messages {
                if !self.validate_server_msg(message.msg_id)? {
                    continue;
                }
                if (message.seqno & 1) == 1 {
                    self.acks.mark(message.msg_id);
                }
                if found.is_none() {
                    found = self.handle_object(&message.body.0, wanted)?;
                }
            }
            if !self.validate_server_msg(opened.msg_id)? {
                return Ok(None);
            }
            Ok(found)
        } else {
            if !self.validate_server_msg(opened.msg_id)? {
                return Ok(None);
            }
            let mut reader = Reader::new(&payload);
            TlObject::read_boxed(&mut reader)?;
            let consumed = reader.position();
            mtproto2::check_padding(payload.len(), consumed)?;
            if (opened.seq_no & 1) == 1 {
                self.acks.mark(opened.msg_id);
            }
            self.handle_object(&payload[..consumed], wanted)
        }
    }

    fn validate_server_msg(&mut self, msg_id: i64) -> Result<bool, KostaError> {
        match self.server_ids.validate(msg_id, now_secs()) {
            Ok(()) => Ok(true),
            Err(ProtocolError::Duplicate) => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn handle_object(&mut self, body: &[u8], wanted: i64) -> Result<Option<TlObject>, KostaError> {
        let object = TlObject::read_boxed(&mut Reader::new(body))?;
        match object {
            TlObject::NewSessionCreated(created) => {
                self.salt = created.server_salt;
                Ok(None)
            }
            TlObject::MsgsAck(_) => Ok(None),
            TlObject::Pong(pong) => Ok(Some(TlObject::Pong(pong))),
            TlObject::RpcResult(result) => {
                if result.req_msg_id != wanted {
                    return Ok(None);
                }
                match *result.result {
                    TlObject::RpcError(error) => Err(KostaError::Protocol(format!(
                        "rpc error {}: {}",
                        error.error_code, error.error_message
                    ))),
                    other => Ok(Some(other)),
                }
            }
            TlObject::BadMsgNotification(notification) => Err(KostaError::Protocol(format!(
                "bad_msg_notification: code {}",
                notification.error_code
            ))),
            other => Ok(Some(other)),
        }
    }

    // ----- Высокоуровневые методы -----

    /// Регистрирует пользователя и запоминает `user_id`.
    pub fn sign_up(&mut self, phone: &str, password: &str) -> Result<i64, KostaError> {
        let result = self.invoke(TlObject::SignUp(SignUp {
            phone: phone.to_string(),
            password: password.to_string(),
        }))?;
        match result {
            TlObject::AuthResult(auth) => {
                self.user_id = Some(auth.user_id);
                Ok(auth.user_id)
            }
            other => Err(unexpected("authResult", &other)),
        }
    }

    /// Создаёт группу и возвращает `dialog_id`.
    pub fn create_group(&mut self, title: &str) -> Result<i64, KostaError> {
        let result = self.invoke(TlObject::CreateGroup(CreateGroup {
            title: title.to_string(),
            participants: Vec::new(),
        }))?;
        dialog_id_from(result)
    }

    /// Находит или создаёт личный диалог с пользователем.
    pub fn create_private_dialog(&mut self, user_id: i64) -> Result<i64, KostaError> {
        let result = self.invoke(TlObject::CreatePrivateDialog(CreatePrivateDialog {
            user_id,
        }))?;
        dialog_id_from(result)
    }

    /// Отправляет сообщение и возвращает созданный `ChatMessage`.
    pub fn send_message(&mut self, chat_id: i64, text: &str) -> Result<ChatMessage, KostaError> {
        let result = self.invoke(TlObject::SendMessage(kosta_tl::SendMessage {
            chat_id,
            text: text.to_string(),
            random_id: random_non_zero_i64(),
        }))?;
        match result {
            TlObject::ChatMessage(message) => Ok(message),
            other => Err(unexpected("chatMessage", &other)),
        }
    }

    /// Возвращает последние сообщения диалога (новые первыми, как их отдаёт сервер).
    pub fn get_history(
        &mut self,
        chat_id: i64,
        offset: i32,
        limit: i32,
    ) -> Result<Vec<ChatMessage>, KostaError> {
        let result = self.invoke(TlObject::GetHistory(GetHistory {
            chat_id,
            offset,
            limit,
        }))?;
        match result {
            TlObject::HistoryResult(HistoryResult { messages, .. }) => Ok(messages),
            other => Err(unexpected("historyResult", &other)),
        }
    }

    /// Возвращает список диалогов пользователя.
    pub fn get_dialogs(&mut self) -> Result<Vec<DialogInfo>, KostaError> {
        let result = self.invoke(TlObject::GetDialogs(GetDialogs))?;
        match result {
            TlObject::DialogList(DialogList { dialogs }) => Ok(dialogs),
            other => Err(unexpected("dialogList", &other)),
        }
    }

    /// Подтверждает доставку сообщения.
    pub fn ack_message(&mut self, message_id: i64) -> Result<(), KostaError> {
        let result = self.invoke(TlObject::AckMessage(kosta_tl::AckMessage { message_id }))?;
        expect_ok(result)
    }

    /// Обновляет собственный статус.
    pub fn update_status(&mut self, status: UserStatus) -> Result<(), KostaError> {
        let user_id = self
            .user_id
            .ok_or_else(|| protocol("user_id is unknown: sign up first"))?;
        let result = self.invoke(TlObject::UpdateUserStatus(UpdateUserStatus {
            user_id,
            status,
        }))?;
        expect_ok(result)
    }
}

fn serialize<T: TlWrite>(value: &T) -> Result<Vec<u8>, KostaError> {
    let mut out = Vec::new();
    value.write_bytes(&mut out)?;
    Ok(out)
}

fn protocol(message: impl Into<String>) -> KostaError {
    KostaError::Protocol(message.into())
}

fn unexpected(expected: &str, actual: &TlObject) -> KostaError {
    protocol(format!(
        "expected {expected}, got object 0x{:08x}",
        actual.constructor_id()
    ))
}

fn dialog_id_from(result: TlObject) -> Result<i64, KostaError> {
    match result {
        TlObject::DialogInfo(DialogInfo { id, .. }) => Ok(id),
        other => Err(unexpected("dialogInfo", &other)),
    }
}

fn expect_ok(result: TlObject) -> Result<(), KostaError> {
    match result {
        TlObject::OkResult(_) => Ok(()),
        other => Err(unexpected("okResult", &other)),
    }
}

fn is_timeout(error: &KostaError) -> bool {
    match error {
        KostaError::Io(io) => matches!(
            io.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
        ),
        KostaError::Transport(TransportError::Io(io)) => matches!(
            io.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
        ),
        _ => false,
    }
}

fn random_int128() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

fn random_int256() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

fn random_non_zero_i64() -> i64 {
    let mut bytes = [0u8; 8];
    OsRng.fill_bytes(&mut bytes);
    let value = i64::from_le_bytes(bytes);
    if value == 0 {
        1
    } else {
        value
    }
}

fn bytes_to_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.len() > 8 {
        return None;
    }
    let mut buffer = [0u8; 8];
    buffer[8 - bytes.len()..].copy_from_slice(bytes);
    Some(u64::from_be_bytes(buffer))
}

fn key_fingerprint(public: &VerifyingKey) -> i64 {
    use sha1::{Digest, Sha1};

    let digest = Sha1::digest(public.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&digest[12..20]);
    i64::from_le_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_to_u64_pads_leading_zeros() {
        assert_eq!(bytes_to_u64(&[]), Some(0));
        assert_eq!(bytes_to_u64(&[1, 2]), Some(0x0102));
        assert_eq!(bytes_to_u64(&[0u8; 8]), Some(0));
        assert_eq!(bytes_to_u64(&[1u8; 9]), None);
    }

    #[test]
    fn random_i64_is_never_zero() {
        for _ in 0..100 {
            assert_ne!(random_non_zero_i64(), 0);
        }
    }

    #[test]
    fn timeout_errors_are_classified() {
        let timed_out = KostaError::Transport(TransportError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timeout",
        )));
        assert!(is_timeout(&timed_out));
        assert!(!is_timeout(&KostaError::Protocol("x".into())));
    }
}
