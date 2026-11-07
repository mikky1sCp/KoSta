//! Обработчик одного TCP-соединения MTProto 2.0.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use tracing::{debug, info, warn};

use kosta_crypto::mtproto2::{self, Direction};
use kosta_protocol::frame;
use kosta_protocol::msg_id::now_secs;
use kosta_protocol::ProtocolError;
use kosta_tl::{
    Bytes, Message, MsgContainer, MsgsAck, NewSessionCreated, Pong, Reader, RpcError, RpcResult,
    TlObject, TlRead, TlWrite,
};
use kosta_transport::mtproto_abridged::{decode_frame, encode_frame, read_init};

use crate::db::Database;
use crate::mtproto::handshake;
use crate::mtproto::rpc;
use crate::mtproto::session::{MtprotoSession, SessionStore};

/// Соединение: обычный TCP или TLS-обёртка.
#[allow(clippy::large_enum_variant)]
pub enum Conn {
    /// Обычный TCP.
    Plain(TcpStream),
    /// TLS поверх TCP.
    Tls(native_tls::TlsStream<TcpStream>),
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

/// Крутит цикл обмена кадрами, пока клиент не закроет соединение.
pub fn handle_connection(
    mut conn: Conn,
    store: Arc<SessionStore>,
    db: Arc<Database>,
    peer: SocketAddr,
    _idle_timeout: Duration,
) -> Result<()> {
    read_init(&mut conn).map_err(|e| anyhow!("abridged init failed: {e}"))?;
    info!("MTProto 2.0 connection from {} established", peer);

    let mut active_session: Option<i64> = None;
    loop {
        let raw = match decode_frame(&mut conn) {
            Ok(raw) => raw,
            Err(kosta_transport::TransportError::Io(error))
                if error.kind() == std::io::ErrorKind::UnexpectedEof =>
            {
                debug!("peer {} closed the connection", peer);
                break;
            }
            Err(error) => {
                warn!("transport error from {}: {error}", peer);
                break;
            }
        };

        match process_frame(&raw, &store, &db, peer) {
            Ok(Some(response)) => {
                if let Some(id) = frame_auth_key_id(&raw) {
                    active_session = Some(id);
                }
                let encoded = encode_frame(&response)?;
                if let Err(error) = conn.write_all(&encoded).and_then(|_| conn.flush()) {
                    warn!("failed to write response to {}: {error}", peer);
                    break;
                }
            }
            Ok(None) => {}
            Err(error) => {
                warn!("closing connection to {}: {error:#}", peer);
                break;
            }
        }
    }

    // Сохраняем счётчики, чтобы после рестарта продолжить сессию.
    if let Some(auth_key_id) = active_session {
        if let Ok(Some(session)) = store.get(auth_key_id) {
            let session = session.lock().unwrap_or_else(|e| e.into_inner());
            if let Err(error) = session.persist(&db) {
                warn!("failed to persist session {auth_key_id}: {error:#}");
            }
        }
    }
    info!("MTProto 2.0 connection from {} closed", peer);
    Ok(())
}

fn frame_auth_key_id(raw: &[u8]) -> Option<i64> {
    if raw.len() < 8 {
        return None;
    }
    let id = i64::from_le_bytes(raw[0..8].try_into().ok()?);
    (id != 0).then_some(id)
}

fn serialize<T: TlWrite>(value: &T) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    value.write_bytes(&mut out)?;
    Ok(out)
}

/// Обрабатывает один кадр: возвращает готовый к отправке payload (или `None`).
fn process_frame(
    raw: &[u8],
    store: &SessionStore,
    db: &Database,
    peer: SocketAddr,
) -> Result<Option<Vec<u8>>> {
    if raw.len() < 8 {
        return Err(anyhow!("frame is too short: {} bytes", raw.len()));
    }
    let auth_key_id = i64::from_le_bytes(
        raw[0..8]
            .try_into()
            .map_err(|_| anyhow!("invalid auth_key_id"))?,
    );

    if auth_key_id == 0 {
        return process_plain_frame(raw, store, peer);
    }
    process_encrypted_frame(raw, auth_key_id, store, db, peer)
}

fn process_plain_frame(
    raw: &[u8],
    store: &SessionStore,
    peer: SocketAddr,
) -> Result<Option<Vec<u8>>> {
    let plain = frame::parse_plain_frame(raw)?;
    let object = TlObject::read_boxed(&mut Reader::new(&plain.body))?;

    // Рукопожатие — дорогая анонимная операция; ограничиваем по IP.
    if handshake::is_expensive_handshake_object(&object) {
        crate::rate_limiter::check_limit(
            &crate::rate_limiter::TCP_HANDSHAKE_LIMITER,
            peer.ip().to_string(),
        )
        .map_err(|error| anyhow!("handshake rate limit exceeded for {}: {error}", peer))?;
    }

    let response = handshake::handle_plain_object(object, store, peer.ip())?;
    let Some(response) = response else {
        return Ok(None);
    };
    let body = serialize(&response)?;
    let msg_id = (now_secs() << 32) | 1;
    Ok(Some(frame::build_plain_frame(msg_id, &body)?))
}

fn process_encrypted_frame(
    raw: &[u8],
    auth_key_id: i64,
    store: &SessionStore,
    db: &Database,
    peer: SocketAddr,
) -> Result<Option<Vec<u8>>> {
    let session_arc = store
        .get(auth_key_id)?
        .ok_or_else(|| anyhow!("unknown auth_key_id {auth_key_id}"))?;
    let mut session = session_arc.lock().unwrap_or_else(|e| e.into_inner());

    let opened = frame::open_frame(
        &session.auth_key,
        Direction::ClientToServer,
        auth_key_id,
        raw,
    )?;

    // Salt валидируем по окну: 0 до первого new_session_created, затем текущий/прошлый.
    if !session.salt_state.is_acceptable(opened.salt) {
        return Err(anyhow!("unexpected salt {}", opened.salt));
    }

    let now = now_secs();
    let mut outgoing: Vec<(i32, Vec<u8>)> = Vec::new();
    let payload = opened.payload.clone();

    let mut reader = Reader::new(&payload);
    let first_id = reader.read_u32()?;

    if first_id == kosta_tl::MSG_CONTAINER_ID {
        let mut container_reader = Reader::new(&payload);
        let container = MsgContainer::read_bytes(&mut container_reader)?;
        let consumed = container_reader.position();
        mtproto2::check_padding(payload.len(), consumed)
            .map_err(|e| anyhow!("bad container padding: {e}"))?;
        if container.messages.is_empty() {
            return Err(anyhow!("empty msg_container"));
        }

        for message in &container.messages {
            handle_incoming_message(
                &mut session,
                db,
                peer,
                message.msg_id,
                message.seqno,
                &message.body.0,
                &mut outgoing,
            )?;
        }
        // Внешний msg_id контейнера проверяем после вложенных: он генерируется позже.
        session
            .validator
            .validate(opened.msg_id, now)
            .map_err(|e| anyhow!("bad container msg_id: {e}"))?;
    } else {
        let mut object_reader = Reader::new(&payload);
        TlObject::read_boxed(&mut object_reader)?;
        let consumed = object_reader.position();
        mtproto2::check_padding(payload.len(), consumed)
            .map_err(|e| anyhow!("bad message padding: {e}"))?;

        handle_incoming_message(
            &mut session,
            db,
            peer,
            opened.msg_id,
            opened.seq_no,
            &payload[..consumed],
            &mut outgoing,
        )?;
    }

    // new_session_created уходит в первом же ответе, чтобы клиент узнал salt.
    let mut sent_new_session = false;
    if !session.salt_sent {
        let created = TlObject::NewSessionCreated(NewSessionCreated {
            first_msg_id: opened.msg_id,
            unique_id: session.unique_id,
            server_salt: session.server_salt,
        });
        outgoing.insert(0, (0, serialize(&created)?));
        session.salt_sent = true;
        let server_salt = session.server_salt;
        session.salt_state.update(server_salt);
        sent_new_session = true;
    }

    let acked = session.acks.drain();
    if !acked.is_empty() {
        let ack = TlObject::MsgsAck(MsgsAck { msg_ids: acked });
        let insert_at = usize::from(sent_new_session);
        outgoing.insert(insert_at, (0, serialize(&ack)?));
    }

    let persist = session.touch();
    if persist {
        if let Err(error) = session.persist(db) {
            warn!(
                "failed to persist session {}: {error:#}",
                session.auth_key_id
            );
        }
    }

    if outgoing.is_empty() {
        return Ok(None);
    }

    let mut messages = Vec::with_capacity(outgoing.len());
    for (seq_no, body) in outgoing {
        let msg_id = session.msg_ids.next(now);
        messages.push(Message {
            msg_id,
            seqno: seq_no,
            bytes: body.len() as i32,
            body: Bytes(body),
        });
    }
    let container = TlObject::MsgContainer(MsgContainer { messages });
    let container_body = serialize(&container)?;

    // msg_id контейнера генерируется после вложенных сообщений.
    let outer_msg_id = session.msg_ids.next(now);
    let frame = frame::seal_frame(
        &session.auth_key,
        Direction::ServerToClient,
        session.auth_key_id,
        opened.session_id,
        session.salt_state.current(),
        outer_msg_id,
        0,
        &container_body,
    )?;
    Ok(Some(frame))
}

fn handle_incoming_message(
    session: &mut MtprotoSession,
    db: &Database,
    peer: SocketAddr,
    msg_id: i64,
    seq_no: i32,
    body: &[u8],
    outgoing: &mut Vec<(i32, Vec<u8>)>,
) -> Result<()> {
    // Повтор: отдаём сохранённый ответ, не выполняя операцию заново.
    if let Some(cached) = session.responses.get(msg_id).map(|bytes| bytes.to_vec()) {
        outgoing.push((session.seq.next_content(), cached));
        return Ok(());
    }

    match session.validator.validate(msg_id, now_secs()) {
        Ok(()) => {}
        Err(ProtocolError::Duplicate) => return Ok(()),
        Err(error) => return Err(anyhow!("bad msg_id {msg_id}: {error}")),
    }
    session.last_client_seq = seq_no;

    let object = TlObject::read_boxed(&mut Reader::new(body))?;
    debug!("MTProto RPC from {}: {:?}", peer, object);

    // Content-related сообщения (нечётный seq_no) нужно подтверждать.
    if (seq_no & 1) == 1 {
        session.acks.mark(msg_id);
    }

    match object {
        TlObject::MsgsAck(_) | TlObject::Pong(_) => {}
        TlObject::Ping(ping) => {
            // ping — сервисный ответ pong, без rpc_result.
            let pong = TlObject::Pong(Pong {
                msg_id,
                ping_id: ping.ping_id,
            });
            outgoing.push((0, serialize(&pong)?));
        }
        TlObject::ResPq(_)
        | TlObject::ServerDhParams(_)
        | TlObject::SetClientDhParamsAnswer(_)
        | TlObject::NewSessionCreated(_)
        | TlObject::RpcResult(_)
        | TlObject::RpcError(_)
        | TlObject::BadMsgNotification(_)
        | TlObject::MsgResendReq(_)
        | TlObject::GzipPacked(_)
        | TlObject::MsgContainer(_)
        | TlObject::Message(_) => {
            return Err(anyhow!("unexpected object in encrypted frame"));
        }
        TlObject::ReqPq(_)
        | TlObject::ReqPqMulti(_)
        | TlObject::ReqDhParams(_)
        | TlObject::SetClientDhParams(_) => {
            return Err(anyhow!("handshake object inside encrypted frame"));
        }
        request => {
            let result = rpc::dispatch(db, session, request, peer.ip());
            let response = match result {
                Ok(object) => RpcResult {
                    req_msg_id: msg_id,
                    result: Box::new(object),
                },
                Err(error) => RpcResult {
                    req_msg_id: msg_id,
                    result: Box::new(TlObject::RpcError(RpcError {
                        error_code: error.code,
                        error_message: error.message,
                    })),
                },
            };
            let bytes = serialize(&TlObject::RpcResult(response))?;
            session.responses.insert(msg_id, bytes.clone());
            outgoing.push((session.seq.next_content(), bytes));
        }
    }
    Ok(())
}
