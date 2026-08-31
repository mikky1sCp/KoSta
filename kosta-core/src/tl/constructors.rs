use super::types::*;
use crate::error::Error;
use std::io::Read;

// =============================================================================
// Существующие ID (без изменений)
// =============================================================================
pub const REQ_PQ_ID: u32 = 0x60469778;
pub const RES_PQ_ID: u32 = 0x9a2b3c4d;
pub const P_Q_INNER_DATA_ID: u32 = 0x83c95aec;
pub const REQ_DH_PARAMS_ID: u32 = 0xd712e4be;
pub const SERVER_DH_PARAMS_OK_ID: u32 = 0xd0e60441;
pub const SERVER_DH_INNER_DATA_ID: u32 = 0xb5890dba;
pub const CLIENT_DH_INNER_DATA_ID: u32 = 0x6643b654;
pub const SET_CLIENT_DH_PARAMS_ID: u32 = 0xf5045f1f;
pub const DH_GEN_OK_ID: u32 = 0x3bcbf734;

// Прикладные ID (старые)
pub const MESSAGE_ID: u32 = 0x9b9c4b5d;
pub const CHAT_ID: u32 = 0x7a8f3c1e;
pub const SEND_MESSAGE_ID: u32 = 0x1a2b3c4d;
pub const SEND_MESSAGE_ACK_ID: u32 = 0x2b3c4d5e;
pub const GET_HISTORY_ID: u32 = 0x3c4d5e6f;
pub const HISTORY_RESULT_ID: u32 = 0x4d5e6f70;
pub const USER_STATUS_ID: u32 = 0x5e6f7081;
pub const SIGN_UP_ID: u32 = 0x8a9b7c6d;

// =============================================================================
// НОВЫЕ ID для диалогов, групп и статусов
// =============================================================================

// Запросы
pub const CREATE_PRIVATE_DIALOG_ID: u32 = 0xa1b2c3d4;
pub const CREATE_GROUP_ID: u32 = 0xb2c3d4e5;
pub const ADD_GROUP_PARTICIPANT_ID: u32 = 0xc3d4e5f6;
pub const REMOVE_GROUP_PARTICIPANT_ID: u32 = 0xd4e5f607;
pub const UPDATE_STATUS_ID: u32 = 0xe5f60718;      // можно использовать существующий, но добавим новый
pub const GET_DIALOGS_ID: u32 = 0xf6071829;        // запрос списка диалогов

// Ответы
pub const DIALOG_INFO_ID: u32 = 0x0718293a;
pub const DIALOG_LIST_ID: u32 = 0x18293a4b;
pub const DIALOG_MESSAGE_ID: u32 = 0x293a4b5c;      // можно использовать существующий Message

// =============================================================================
// Существующие структуры (без изменений)
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReqPq {
    pub nonce: Int128,
}
impl TlWrite for ReqPq {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.nonce.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for ReqPq {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let nonce = Int128::read_bytes(reader)?;
        Ok(ReqPq { nonce })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResPQ {
    pub nonce: Int128,
    pub server_nonce: Int128,
    pub pq: Vec<u8>,
    pub p: Vec<u8>,
    pub q: Vec<u8>,
    pub server_public_key_fingerprints: Vec<i64>,
}
impl TlWrite for ResPQ {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.nonce.write_bytes(writer)?;
        self.server_nonce.write_bytes(writer)?;
        self.pq.write_bytes(writer)?;
        self.p.write_bytes(writer)?;
        self.q.write_bytes(writer)?;
        self.server_public_key_fingerprints.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for ResPQ {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let nonce = Int128::read_bytes(reader)?;
        let server_nonce = Int128::read_bytes(reader)?;
        let pq = Vec::<u8>::read_bytes(reader)?;
        let p = Vec::<u8>::read_bytes(reader)?;
        let q = Vec::<u8>::read_bytes(reader)?;
        let fingerprints = Vec::<i64>::read_bytes(reader)?;
        Ok(ResPQ {
            nonce,
            server_nonce,
            pq,
            p,
            q,
            server_public_key_fingerprints: fingerprints,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PqInnerData {
    pub pq: Vec<u8>,
    pub p: Vec<u8>,
    pub q: Vec<u8>,
    pub nonce: Int128,
    pub server_nonce: Int128,
    pub new_nonce: Int256,
}
impl TlWrite for PqInnerData {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.pq.write_bytes(writer)?;
        self.p.write_bytes(writer)?;
        self.q.write_bytes(writer)?;
        self.nonce.write_bytes(writer)?;
        self.server_nonce.write_bytes(writer)?;
        self.new_nonce.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for PqInnerData {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(PqInnerData {
            pq: Vec::<u8>::read_bytes(reader)?,
            p: Vec::<u8>::read_bytes(reader)?,
            q: Vec::<u8>::read_bytes(reader)?,
            nonce: Int128::read_bytes(reader)?,
            server_nonce: Int128::read_bytes(reader)?,
            new_nonce: Int256::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReqDHParams {
    pub nonce: Int128,
    pub server_nonce: Int128,
    pub p: Vec<u8>,
    pub q: Vec<u8>,
    pub public_key_fingerprint: i64,
    pub encrypted_data: Vec<u8>,
}
impl TlWrite for ReqDHParams {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.nonce.write_bytes(writer)?;
        self.server_nonce.write_bytes(writer)?;
        self.p.write_bytes(writer)?;
        self.q.write_bytes(writer)?;
        self.public_key_fingerprint.write_bytes(writer)?;
        self.encrypted_data.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for ReqDHParams {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(ReqDHParams {
            nonce: Int128::read_bytes(reader)?,
            server_nonce: Int128::read_bytes(reader)?,
            p: Vec::<u8>::read_bytes(reader)?,
            q: Vec::<u8>::read_bytes(reader)?,
            public_key_fingerprint: i64::read_bytes(reader)?,
            encrypted_data: Vec::<u8>::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerDHParamsOk {
    pub nonce: Int128,
    pub server_nonce: Int128,
    pub encrypted_answer: Vec<u8>,
    pub signature: Vec<u8>,
}
impl TlWrite for ServerDHParamsOk {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.nonce.write_bytes(writer)?;
        self.server_nonce.write_bytes(writer)?;
        self.encrypted_answer.write_bytes(writer)?;
        self.signature.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for ServerDHParamsOk {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(ServerDHParamsOk {
            nonce: Int128::read_bytes(reader)?,
            server_nonce: Int128::read_bytes(reader)?,
            encrypted_answer: Vec::<u8>::read_bytes(reader)?,
            signature: Vec::<u8>::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerDHInnerData {
    pub nonce: Int128,
    pub server_nonce: Int128,
    pub g: i32,
    pub dh_prime: Vec<u8>,
    pub g_a: Vec<u8>,
    pub server_time: i32,
}
impl TlWrite for ServerDHInnerData {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.nonce.write_bytes(writer)?;
        self.server_nonce.write_bytes(writer)?;
        self.g.write_bytes(writer)?;
        self.dh_prime.write_bytes(writer)?;
        self.g_a.write_bytes(writer)?;
        self.server_time.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for ServerDHInnerData {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(ServerDHInnerData {
            nonce: Int128::read_bytes(reader)?,
            server_nonce: Int128::read_bytes(reader)?,
            g: i32::read_bytes(reader)?,
            dh_prime: Vec::<u8>::read_bytes(reader)?,
            g_a: Vec::<u8>::read_bytes(reader)?,
            server_time: i32::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientDHInnerData {
    pub nonce: Int128,
    pub server_nonce: Int128,
    pub retry_id: i64,
    pub g_b: Vec<u8>,
}
impl TlWrite for ClientDHInnerData {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.nonce.write_bytes(writer)?;
        self.server_nonce.write_bytes(writer)?;
        self.retry_id.write_bytes(writer)?;
        self.g_b.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for ClientDHInnerData {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(ClientDHInnerData {
            nonce: Int128::read_bytes(reader)?,
            server_nonce: Int128::read_bytes(reader)?,
            retry_id: i64::read_bytes(reader)?,
            g_b: Vec::<u8>::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetClientDHParams {
    pub nonce: Int128,
    pub server_nonce: Int128,
    pub encrypted_data: Vec<u8>,
}
impl TlWrite for SetClientDHParams {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.nonce.write_bytes(writer)?;
        self.server_nonce.write_bytes(writer)?;
        self.encrypted_data.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for SetClientDHParams {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(SetClientDHParams {
            nonce: Int128::read_bytes(reader)?,
            server_nonce: Int128::read_bytes(reader)?,
            encrypted_data: Vec::<u8>::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DHGenOk {
    pub nonce: Int128,
    pub server_nonce: Int128,
    pub new_nonce_hash1: Int128,
}
impl TlWrite for DHGenOk {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.nonce.write_bytes(writer)?;
        self.server_nonce.write_bytes(writer)?;
        self.new_nonce_hash1.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for DHGenOk {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(DHGenOk {
            nonce: Int128::read_bytes(reader)?,
            server_nonce: Int128::read_bytes(reader)?,
            new_nonce_hash1: Int128::read_bytes(reader)?,
        })
    }
}

// =============================================================================
// ПРИКЛАДНЫЕ СТРУКТУРЫ (существующие)
// =============================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub id: MessageId,
    pub chat_id: ChatId,
    pub sender_id: UserId,
    pub text: String,
    pub timestamp: i64,
    pub is_outgoing: bool,
    pub read: bool,
    pub delivered: bool,
}
impl TlWrite for Message {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.id.write_bytes(writer)?;
        self.chat_id.write_bytes(writer)?;
        self.sender_id.write_bytes(writer)?;
        self.text.write_bytes(writer)?;
        self.timestamp.write_bytes(writer)?;
        self.is_outgoing.write_bytes(writer)?;
        self.read.write_bytes(writer)?;
        self.delivered.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for Message {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(Message {
            id: MessageId::read_bytes(reader)?,
            chat_id: ChatId::read_bytes(reader)?,
            sender_id: UserId::read_bytes(reader)?,
            text: String::read_bytes(reader)?,
            timestamp: i64::read_bytes(reader)?,
            is_outgoing: bool::read_bytes(reader)?,
            read: bool::read_bytes(reader)?,
            delivered: bool::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chat {
    pub id: ChatId,
    pub title: String,
    pub participants: Vec<UserId>,
    pub is_group: bool,
}
impl TlWrite for Chat {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.id.write_bytes(writer)?;
        self.title.write_bytes(writer)?;
        self.participants.write_bytes(writer)?;
        self.is_group.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for Chat {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(Chat {
            id: ChatId::read_bytes(reader)?,
            title: String::read_bytes(reader)?,
            participants: Vec::<UserId>::read_bytes(reader)?,
            is_group: bool::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessage {
    pub chat_id: ChatId,
    pub text: String,
    pub random_id: i64,
}
impl TlWrite for SendMessage {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.chat_id.write_bytes(writer)?;
        self.text.write_bytes(writer)?;
        self.random_id.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for SendMessage {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(SendMessage {
            chat_id: ChatId::read_bytes(reader)?,
            text: String::read_bytes(reader)?,
            random_id: i64::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendMessageAck {
    pub message_id: MessageId,
}
impl TlWrite for SendMessageAck {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.message_id.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for SendMessageAck {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(SendMessageAck {
            message_id: MessageId::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetHistory {
    pub chat_id: ChatId,
    pub offset: i32,
    pub limit: i32,
}
impl TlWrite for GetHistory {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.chat_id.write_bytes(writer)?;
        self.offset.write_bytes(writer)?;
        self.limit.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for GetHistory {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(GetHistory {
            chat_id: ChatId::read_bytes(reader)?,
            offset: i32::read_bytes(reader)?,
            limit: i32::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryResult {
    pub messages: Vec<Message>,
    pub total_count: i32,
}
impl TlWrite for HistoryResult {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.messages.write_bytes(writer)?;
        self.total_count.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for HistoryResult {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(HistoryResult {
            messages: Vec::<Message>::read_bytes(reader)?,
            total_count: i32::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserStatusUpdate {
    pub user_id: UserId,
    pub status: UserStatus,
}
impl TlWrite for UserStatusUpdate {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.user_id.write_bytes(writer)?;
        self.status.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for UserStatusUpdate {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(UserStatusUpdate {
            user_id: UserId::read_bytes(reader)?,
            status: UserStatus::read_bytes(reader)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignUp {
    pub phone: String,
    pub password: String,
}
impl TlWrite for SignUp {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.phone.write_bytes(writer)?;
        self.password.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for SignUp {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(SignUp {
            phone: String::read_bytes(reader)?,
            password: String::read_bytes(reader)?,
        })
    }
}

// =============================================================================
// НОВЫЕ СТРУКТУРЫ ДЛЯ ДИАЛОГОВ, ГРУПП, СТАТУСОВ
// =============================================================================

// Вспомогательный тип – информация о диалоге
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogInfo {
    pub id: DialogId,
    pub title: String,
    pub is_group: bool,
    pub participants: Vec<UserId>,
    pub last_message: Option<Message>,
    pub unread_count: i32,
}
impl TlWrite for DialogInfo {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.id.write_bytes(writer)?;
        self.title.write_bytes(writer)?;
        self.is_group.write_bytes(writer)?;
        self.participants.write_bytes(writer)?;
        self.last_message.write_bytes(writer)?;
        self.unread_count.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for DialogInfo {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(DialogInfo {
            id: DialogId::read_bytes(reader)?,
            title: String::read_bytes(reader)?,
            is_group: bool::read_bytes(reader)?,
            participants: Vec::<UserId>::read_bytes(reader)?,
            last_message: Option::<Message>::read_bytes(reader)?,
            unread_count: i32::read_bytes(reader)?,
        })
    }
}

// Список диалогов
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogList {
    pub dialogs: Vec<DialogInfo>,
}
impl TlWrite for DialogList {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.dialogs.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for DialogList {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(DialogList {
            dialogs: Vec::<DialogInfo>::read_bytes(reader)?,
        })
    }
}

// Запрос на создание приватного диалога
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreatePrivateDialog {
    pub user_id: UserId,
}
impl TlWrite for CreatePrivateDialog {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.user_id.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for CreatePrivateDialog {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(CreatePrivateDialog {
            user_id: UserId::read_bytes(reader)?,
        })
    }
}

// Запрос на создание группы
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateGroup {
    pub title: String,
    pub participants: Vec<UserId>,
}
impl TlWrite for CreateGroup {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.title.write_bytes(writer)?;
        self.participants.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for CreateGroup {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(CreateGroup {
            title: String::read_bytes(reader)?,
            participants: Vec::<UserId>::read_bytes(reader)?,
        })
    }
}

// Добавление участника в группу
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddGroupParticipant {
    pub dialog_id: DialogId,
    pub user_id: UserId,
}
impl TlWrite for AddGroupParticipant {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.dialog_id.write_bytes(writer)?;
        self.user_id.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for AddGroupParticipant {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(AddGroupParticipant {
            dialog_id: DialogId::read_bytes(reader)?,
            user_id: UserId::read_bytes(reader)?,
        })
    }
}

// Удаление участника из группы
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoveGroupParticipant {
    pub dialog_id: DialogId,
    pub user_id: UserId,
}
impl TlWrite for RemoveGroupParticipant {
    fn write_bytes<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        self.dialog_id.write_bytes(writer)?;
        self.user_id.write_bytes(writer)?;
        Ok(())
    }
}
impl TlRead for RemoveGroupParticipant {
    fn read_bytes<R: Read>(reader: &mut R) -> Result<Self, Error> {
        Ok(RemoveGroupParticipant {
            dialog_id: DialogId::read_bytes(reader)?,
            user_id: UserId::read_bytes(reader)?,
        })
    }
}

// Запрос списка диалогов
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GetDialogs {
    // можно добавить offset/limit позже
}
impl TlWrite for GetDialogs {
    fn write_bytes<W: std::io::Write>(&self, _writer: &mut W) -> Result<(), Error> {
        // пустой запрос
        Ok(())
    }
}
impl TlRead for GetDialogs {
    fn read_bytes<R: Read>(_reader: &mut R) -> Result<Self, Error> {
        Ok(GetDialogs {})
    }
}

// Обновление статуса (переиспользуем существующий UserStatusUpdate или создадим новый)
// Мы уже имеем UserStatusUpdate, так что отдельный не нужен.

// =============================================================================
// Универсальный контейнер TlObject
// =============================================================================

#[derive(Debug, Clone)]
pub enum TlObject {
    // Существующие
    ReqPq(ReqPq),
    ResPQ(ResPQ),
    PqInnerData(PqInnerData),
    ReqDHParams(ReqDHParams),
    ServerDHParamsOk(ServerDHParamsOk),
    ServerDHInnerData(ServerDHInnerData),
    ClientDHInnerData(ClientDHInnerData),
    SetClientDHParams(SetClientDHParams),
    DHGenOk(DHGenOk),
    Message(Message),
    Chat(Chat),
    SendMessage(SendMessage),
    SendMessageAck(SendMessageAck),
    GetHistory(GetHistory),
    HistoryResult(HistoryResult),
    UserStatusUpdate(UserStatusUpdate),
    SignUp(SignUp),

    // НОВЫЕ
    CreatePrivateDialog(CreatePrivateDialog),
    CreateGroup(CreateGroup),
    AddGroupParticipant(AddGroupParticipant),
    RemoveGroupParticipant(RemoveGroupParticipant),
    GetDialogs(GetDialogs),
    DialogInfo(DialogInfo),
    DialogList(DialogList),
}

impl TlObject {
    pub fn write_boxed<W: std::io::Write>(&self, writer: &mut W) -> Result<(), Error> {
        match self {
            // Существующие (без изменений)
            TlObject::ReqPq(obj) => {
                (REQ_PQ_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::ResPQ(obj) => {
                (RES_PQ_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::PqInnerData(obj) => {
                (P_Q_INNER_DATA_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::ReqDHParams(obj) => {
                (REQ_DH_PARAMS_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::ServerDHParamsOk(obj) => {
                (SERVER_DH_PARAMS_OK_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::ServerDHInnerData(obj) => {
                (SERVER_DH_INNER_DATA_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::ClientDHInnerData(obj) => {
                (CLIENT_DH_INNER_DATA_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::SetClientDHParams(obj) => {
                (SET_CLIENT_DH_PARAMS_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::DHGenOk(obj) => {
                (DH_GEN_OK_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::Message(obj) => {
                (MESSAGE_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::Chat(obj) => {
                (CHAT_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::SendMessage(obj) => {
                (SEND_MESSAGE_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::SendMessageAck(obj) => {
                (SEND_MESSAGE_ACK_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::GetHistory(obj) => {
                (GET_HISTORY_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::HistoryResult(obj) => {
                (HISTORY_RESULT_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::UserStatusUpdate(obj) => {
                (USER_STATUS_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::SignUp(obj) => {
                (SIGN_UP_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }

            // НОВЫЕ
            TlObject::CreatePrivateDialog(obj) => {
                (CREATE_PRIVATE_DIALOG_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::CreateGroup(obj) => {
                (CREATE_GROUP_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::AddGroupParticipant(obj) => {
                (ADD_GROUP_PARTICIPANT_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::RemoveGroupParticipant(obj) => {
                (REMOVE_GROUP_PARTICIPANT_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::GetDialogs(obj) => {
                (GET_DIALOGS_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::DialogInfo(obj) => {
                (DIALOG_INFO_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
            TlObject::DialogList(obj) => {
                (DIALOG_LIST_ID as i32).write_bytes(writer)?;
                obj.write_bytes(writer)?;
            }
        }
        Ok(())
    }

    pub fn read_boxed<R: Read>(reader: &mut R) -> Result<Self, Error> {
        let constructor_id = u32::read_bytes(reader)?;
        match constructor_id {
            // Существующие
            REQ_PQ_ID => Ok(TlObject::ReqPq(ReqPq::read_bytes(reader)?)),
            RES_PQ_ID => Ok(TlObject::ResPQ(ResPQ::read_bytes(reader)?)),
            P_Q_INNER_DATA_ID => Ok(TlObject::PqInnerData(PqInnerData::read_bytes(reader)?)),
            REQ_DH_PARAMS_ID => Ok(TlObject::ReqDHParams(ReqDHParams::read_bytes(reader)?)),
            SERVER_DH_PARAMS_OK_ID => Ok(TlObject::ServerDHParamsOk(ServerDHParamsOk::read_bytes(reader)?)),
            SERVER_DH_INNER_DATA_ID => Ok(TlObject::ServerDHInnerData(ServerDHInnerData::read_bytes(reader)?)),
            CLIENT_DH_INNER_DATA_ID => Ok(TlObject::ClientDHInnerData(ClientDHInnerData::read_bytes(reader)?)),
            SET_CLIENT_DH_PARAMS_ID => Ok(TlObject::SetClientDHParams(SetClientDHParams::read_bytes(reader)?)),
            DH_GEN_OK_ID => Ok(TlObject::DHGenOk(DHGenOk::read_bytes(reader)?)),
            MESSAGE_ID => Ok(TlObject::Message(Message::read_bytes(reader)?)),
            CHAT_ID => Ok(TlObject::Chat(Chat::read_bytes(reader)?)),
            SEND_MESSAGE_ID => Ok(TlObject::SendMessage(SendMessage::read_bytes(reader)?)),
            SEND_MESSAGE_ACK_ID => Ok(TlObject::SendMessageAck(SendMessageAck::read_bytes(reader)?)),
            GET_HISTORY_ID => Ok(TlObject::GetHistory(GetHistory::read_bytes(reader)?)),
            HISTORY_RESULT_ID => Ok(TlObject::HistoryResult(HistoryResult::read_bytes(reader)?)),
            USER_STATUS_ID => Ok(TlObject::UserStatusUpdate(UserStatusUpdate::read_bytes(reader)?)),
            SIGN_UP_ID => Ok(TlObject::SignUp(SignUp::read_bytes(reader)?)),

            // НОВЫЕ
            CREATE_PRIVATE_DIALOG_ID => Ok(TlObject::CreatePrivateDialog(CreatePrivateDialog::read_bytes(reader)?)),
            CREATE_GROUP_ID => Ok(TlObject::CreateGroup(CreateGroup::read_bytes(reader)?)),
            ADD_GROUP_PARTICIPANT_ID => Ok(TlObject::AddGroupParticipant(AddGroupParticipant::read_bytes(reader)?)),
            REMOVE_GROUP_PARTICIPANT_ID => Ok(TlObject::RemoveGroupParticipant(RemoveGroupParticipant::read_bytes(reader)?)),
            GET_DIALOGS_ID => Ok(TlObject::GetDialogs(GetDialogs::read_bytes(reader)?)),
            DIALOG_INFO_ID => Ok(TlObject::DialogInfo(DialogInfo::read_bytes(reader)?)),
            DIALOG_LIST_ID => Ok(TlObject::DialogList(DialogList::read_bytes(reader)?)),

            _ => Err(Error::InvalidConstructor(constructor_id)),
        }
    }
}