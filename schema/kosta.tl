// KoSta TL schema — собственный сабсет TL для MTProto 2.0.
//
// Правила:
//   * секции `---types---` и `---functions---`;
//   * объявление: `name#id? arg:type ... = ResultType;`;
//   * id опционален: без него генератор считает CRC32 от нормализованного
//     объявления (свои прикладные конструкторы);
//   * канонические объекты MTProto (хендшейк, конверт) объявлены с явными
//     id из спецификации;
//   * типы полей: int, long, int128, int256, string, bytes, Bool, Object,
//     vector<T> и объявленные bare-типы (например, ChatMessage).
//
// Отличия от Telegram TL (осознанные, см. docs/mtproto-migration.md):
//   * свой namespace прикладных RPC (sendMessage, getHistory, ...);
//   * в server_DH_params_ok есть отдельное поле signature (Ed25519 вместо
//     RSA-подписи внутри encrypted_answer);
//   * pq 64-битный, p/q в resPQ не передаются (этап 4).

---types---

// ---------------------------------------------------------------------------
// Хендшейк (MTProto auth key generation)
// ---------------------------------------------------------------------------

resPQ#05162463 nonce:int128 server_nonce:int128 pq:bytes server_public_key_fingerprints:vector<long> = ResPQ;

p_q_inner_data#83c95aec pq:bytes p:bytes q:bytes nonce:int128 server_nonce:int128 new_nonce:int256 = P_Q_inner_data;

server_DH_params_ok#d0e60441 nonce:int128 server_nonce:int128 encrypted_answer:bytes signature:bytes = Server_DH_Params;
server_DH_params_fail#79cb045d nonce:int128 server_nonce:int128 new_nonce_hash:int128 = Server_DH_Params;

server_DH_inner_data#b5890dba nonce:int128 server_nonce:int128 g:int dh_prime:bytes g_a:bytes server_time:int = Server_DH_inner_data;

client_DH_inner_data#6643b654 nonce:int128 server_nonce:int128 retry_id:long g_b:bytes = Client_DH_inner_data;

dh_gen_ok#3bcbf734 nonce:int128 server_nonce:int128 new_nonce_hash1:int128 = Set_client_DH_params_answer;
dh_gen_fail#a69dae02 nonce:int128 server_nonce:int128 new_nonce_hash3:int128 = Set_client_DH_params_answer;

// ---------------------------------------------------------------------------
// Конверт MTProto 2.0
// ---------------------------------------------------------------------------

msg_container#73f1f8dc messages:vector<Message> = MsgContainer;

message#5bb8e511 msg_id:long seqno:int bytes:int body:bytes = Message;

msgs_ack#62d6b459 msg_ids:vector<long> = MsgsAck;

rpc_result#f35c6d01 req_msg_id:long result:Object = RpcResult;
rpc_error#2144ca19 error_code:int error_message:string = RpcError;

bad_msg_notification#a7eff811 bad_msg_id:long bad_msg_seqno:int error_code:int = BadMsgNotification;
msg_resend_req#7d861a08 msg_ids:vector<long> = MsgResendReq;
new_session_created#9ec20908 first_msg_id:long unique_id:long server_salt:long = NewSessionCreated;
pong#347773c5 msg_id:long ping_id:long = Pong;
gzip_packed#3072cfa1 packed_data:bytes = GzipPacked;

// ---------------------------------------------------------------------------
// Прикладные типы KoSta
// ---------------------------------------------------------------------------

chatMessage id:long chat_id:long sender_id:long text:string timestamp:long is_outgoing:Bool read:Bool delivered:Bool = ChatMessage;
chatMessageEmpty = ChatMessage;

authResult user_id:long auth_key_id:long = AuthResult;

dialogInfo id:long title:string is_group:Bool participants:vector<long> last_message:ChatMessage unread_count:int = DialogInfo;
dialogList dialogs:vector<DialogInfo> = DialogList;
historyResult messages:vector<ChatMessage> total_count:int = HistoryResult;

userStatusOnline = UserStatus;
userStatusOffline = UserStatus;
userStatusTyping = UserStatus;
userStatusRecently = UserStatus;
userStatusLastWeek = UserStatus;
userStatusLastMonth = UserStatus;

// Универсальный «успешный» ответ для RPC без полезной нагрузки.
ok = OkResult;

---functions---

// ---------------------------------------------------------------------------
// Хендшейк и сервисные запросы
// ---------------------------------------------------------------------------

req_pq#60469778 nonce:int128 = ResPQ;
req_pq_multi#be7e8ef1 nonce:int128 = ResPQ;
req_DH_params#d712e4be nonce:int128 server_nonce:int128 p:bytes q:bytes public_key_fingerprint:long encrypted_data:bytes = Server_DH_Params;
set_client_DH_params#f5045f1f nonce:int128 server_nonce:int128 encrypted_data:bytes = Set_client_DH_params_answer;
ping#7abe77ec ping_id:long = Pong;

// ---------------------------------------------------------------------------
// Прикладные RPC (id считаются генератором)
// ---------------------------------------------------------------------------

signUp phone:string password:string = AuthResult;
sendMessage chat_id:long text:string random_id:long = ChatMessage;
getHistory chat_id:long offset:int limit:int = HistoryResult;
ackMessage message_id:long = OkResult;
updateUserStatus user_id:long status:UserStatus = OkResult;
getDialogs = DialogList;
createPrivateDialog user_id:long = DialogInfo;
createGroup title:string participants:vector<long> = DialogInfo;
addGroupParticipant dialog_id:long user_id:long = DialogInfo;
removeGroupParticipant dialog_id:long user_id:long = DialogInfo;