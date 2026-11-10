#!/usr/bin/env node
// E2E для ответов, редактирования и удаления сообщений.
//
//   node scripts/messages.mjs
//
// Требует запущенный сервер. Для прогона локально удобно поднять его с
// отключёнными лимитами и отдельной базой:
//
//   KOSTA_RATE_LIMIT_DISABLED=1 KOSTA_DB_PATH=e2e.db cargo run -p kosta-server

const BASE = process.env.KOSTA_BASE ?? 'http://127.0.0.1:8081';
const WS_URL = BASE.replace(/^http/, 'ws') + '/ws';
const PASSWORD = process.env.KOSTA_PASSWORD ?? 'testpass123';

let passed = 0;
const failures = [];

function check(name, ok, detail = '') {
  if (ok) {
    passed += 1;
    console.log(`  ok   ${name}`);
  } else {
    failures.push(name);
    console.log(`  FAIL ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

async function req(path, { method = 'GET', token, body } = {}) {
  const headers = {};
  if (token) headers.Authorization = `Bearer ${token}`;
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  const res = await fetch(BASE + path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(15000),
  });
  const text = await res.text();
  let data = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = text;
  }
  return { status: res.status, body: data };
}

/** WS-сессия с очередью входящих кадров. */
function wsSession() {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(WS_URL);
    const queue = [];
    const waiters = [];
    ws.addEventListener('error', () => reject(new Error('websocket error')));
    ws.addEventListener('message', (ev) => {
      let msg;
      try {
        msg = JSON.parse(ev.data);
      } catch {
        return;
      }
      const waiter = waiters.shift();
      if (waiter) waiter(msg);
      else queue.push(msg);
    });
    ws.addEventListener('open', () => {
      resolve({
        send: (obj) => ws.send(JSON.stringify(obj)),
        next: (pred = () => true, timeoutMs = 4000) =>
          new Promise((res, rej) => {
            const deadline = Date.now() + timeoutMs;
            const tryDeliver = (msg) => {
              if (pred(msg)) {
                res(msg);
                return true;
              }
              return false;
            };
            // Сначала смотрим очередь.
            for (let i = 0; i < queue.length; i += 1) {
              if (tryDeliver(queue[i])) {
                queue.splice(i, 1);
                return;
              }
            }
            const timer = setInterval(() => {
              for (let i = 0; i < queue.length; i += 1) {
                if (tryDeliver(queue[i])) {
                  queue.splice(i, 1);
                  clearInterval(timer);
                  clearTimeout(timeout);
                  return;
                }
              }
              if (Date.now() > deadline) {
                clearInterval(timer);
                clearTimeout(timeout);
                rej(new Error('timeout waiting for ws message'));
              }
            }, 20);
            const timeout = setTimeout(() => {}, timeoutMs);
          }),
        close: () => ws.close(),
      });
    });
  });
}

async function connect(token) {
  const ws = await wsSession();
  await ws.next((m) => m.type === 'connected');
  ws.send({ action: 'auth', payload: { token } });
  const auth = await ws.next((m) => m.type === 'auth_success' || m.type === 'auth_failed');
  if (auth.type !== 'auth_success') throw new Error('ws auth failed');
  return ws;
}

console.log(`KoSta messages e2e -> ${BASE}\n`);

const stamp = String(Date.now()).slice(-7);
const phoneA = `+7995${stamp}`;
const phoneB = `+7996${stamp}`;

const loginA = await req('/api/login', {
  method: 'POST',
  body: { phone: phoneA, password: PASSWORD, register: true },
});
const loginB = await req('/api/login', {
  method: 'POST',
  body: { phone: phoneB, password: PASSWORD, register: true },
});
const tokenA = loginA.body?.token;
const tokenB = loginB.body?.token;
const userA = loginA.body?.user_id;
const userB = loginB.body?.user_id;
check('два пользователя созданы', Boolean(tokenA && tokenB));

const chat = await req('/api/chats/private', {
  method: 'POST',
  token: tokenA,
  body: { phone: phoneB },
});
const dialogId = chat.body?.chat?.id;
check('личный чат создан', Boolean(dialogId), JSON.stringify(chat.body));

const wsA = await connect(tokenA);
const wsB = await connect(tokenB);
wsA.send({ action: 'subscribe', payload: { dialog_id: dialogId } });
wsB.send({ action: 'subscribe', payload: { dialog_id: dialogId } });
await wsA.next((m) => m.type === 'subscribed');
await wsB.next((m) => m.type === 'subscribed');

// ---------- 1. Ответ с цитатой ----------
const helloText = `hello ${stamp}`;
wsA.send({ action: 'send_message', payload: { dialog_id: dialogId, text: helloText } });
const helloEcho = await wsA.next((m) => m.type === 'new_message' && m.payload?.text === helloText);
const helloId = helloEcho.payload.id;
check('исходное сообщение отправлено', Number.isFinite(helloId));

const replyText = `reply ${stamp}`;
wsB.send({
  action: 'send_message',
  payload: { dialog_id: dialogId, text: replyText, reply_to_id: helloId },
});
const replyEcho = await wsB.next((m) => m.type === 'new_message' && m.payload?.text === replyText);
check(
  'ответ содержит id цитируемого сообщения',
  replyEcho.payload.reply_to_id === helloId,
  JSON.stringify(replyEcho.payload),
);
check(
  'ответ содержит текст цитаты',
  replyEcho.payload.reply_to_text === helloText,
  JSON.stringify(replyEcho.payload.reply_to_text),
);
check(
  'ответ содержит автора цитаты',
  replyEcho.payload.reply_to_sender_id === userA,
  String(replyEcho.payload.reply_to_sender_id),
);

const incomingReply = await wsA.next(
  (m) => m.type === 'new_message' && m.payload?.text === replyText,
);
check('получатель видит цитату в событии', incomingReply.payload.reply_to_id === helloId);

const history = await req(`/api/history?chat_id=${dialogId}&limit=20`, { token: tokenA });
const replyInHistory = Array.isArray(history.body)
  ? history.body.find((m) => m.text === replyText)
  : null;
check(
  'история отдаёт цитату',
  replyInHistory?.reply_to_id === helloId &&
    replyInHistory?.reply_to_text === helloText &&
    replyInHistory?.reply_to_sender_id === userA,
  JSON.stringify(replyInHistory),
);

// ---------- 2. Ответ на сообщение из чужого диалога запрещён ----------
const phoneC = `+7997${stamp}`;
const loginC = await req('/api/login', {
  method: 'POST',
  body: { phone: phoneC, password: PASSWORD, register: true },
});
const tokenC = loginC.body?.token;
const foreignChat = await req('/api/chats/private', {
  method: 'POST',
  token: tokenC,
  body: { phone: phoneA },
});
const foreignDialogId = foreignChat.body?.chat?.id;
const wsC = await connect(tokenC);
wsC.send({ action: 'subscribe', payload: { dialog_id: foreignDialogId } });
await wsC.next((m) => m.type === 'subscribed');
wsC.send({
  action: 'send_message',
  payload: { dialog_id: foreignDialogId, text: 'foreign', reply_to_id: helloId },
});
const replyDenied = await wsC.next(
  (m) => m.type === 'error' || m.type === 'new_message',
  4000,
).catch(() => null);
check(
  'нельзя ответить на сообщение из чужого диалога',
  replyDenied?.type === 'error',
  JSON.stringify(replyDenied),
);

// ---------- 3. Редактирование ----------
const editedText = `hello edited ${stamp}`;
const editRes = await req(`/api/messages/${helloId}`, {
  method: 'PATCH',
  token: tokenA,
  body: { text: editedText },
});
check('PATCH /api/messages/{id} -> 200', editRes.status === 200, `status ${editRes.status}`);

const editedEvent = await wsB
  .next((m) => m.type === 'message_edited' && m.payload?.message_id === helloId, 4000)
  .catch(() => null);
check(
  'подписчик получил message_edited',
  editedEvent?.payload?.text === editedText && editedEvent?.payload?.dialog_id === dialogId,
  JSON.stringify(editedEvent?.payload),
);

const afterEdit = await req(`/api/history?chat_id=${dialogId}&limit=20`, { token: tokenA });
const editedRow = Array.isArray(afterEdit.body)
  ? afterEdit.body.find((m) => m.id === helloId)
  : null;
check(
  'история отдаёт новый текст и edited_at',
  editedRow?.text === editedText && Number.isFinite(editedRow?.edited_at),
  JSON.stringify(editedRow),
);

const foreignEdit = await req(`/api/messages/${helloId}`, {
  method: 'PATCH',
  token: tokenB,
  body: { text: 'hacked' },
});
check('чужое сообщение редактировать нельзя -> 403', foreignEdit.status === 403, `status ${foreignEdit.status}`);

const emptyEdit = await req(`/api/messages/${helloId}`, {
  method: 'PATCH',
  token: tokenA,
  body: { text: '   ' },
});
check('пустой текст правки -> 400', emptyEdit.status === 400, `status ${emptyEdit.status}`);

// ---------- 4. Удаление ----------
const replyId = replyEcho.payload.id;
const delRes = await req(`/api/messages/${replyId}`, { method: 'DELETE', token: tokenB });
check('DELETE /api/messages/{id} -> 200', delRes.status === 200, `status ${delRes.status}`);

const deletedEvent = await wsA
  .next((m) => m.type === 'message_deleted' && m.payload?.message_id === replyId, 4000)
  .catch(() => null);
check(
  'подписчик получил message_deleted',
  deletedEvent?.payload?.message_id === replyId &&
    deletedEvent?.payload?.dialog_id === dialogId,
  JSON.stringify(deletedEvent?.payload),
);

const afterDelete = await req(`/api/history?chat_id=${dialogId}&limit=20`, { token: tokenA });
const stillThere = Array.isArray(afterDelete.body)
  ? afterDelete.body.some((m) => m.id === replyId)
  : true;
check('удалённое сообщение пропало из истории', !stillThere);

const foreignDelete = await req(`/api/messages/${helloId}`, {
  method: 'DELETE',
  token: tokenB,
});
check('чужое сообщение удалять нельзя -> 403', foreignDelete.status === 403, `status ${foreignDelete.status}`);

const missingEdit = await req('/api/messages/99999999', {
  method: 'PATCH',
  token: tokenA,
  body: { text: 'x' },
});
check('правка несуществующего сообщения -> 404', missingEdit.status === 404, `status ${missingEdit.status}`);

// ---------- 6. Реакции ----------
const react = (token, messageId, emoji) =>
  req(`/api/messages/${messageId}/reactions`, {
    method: 'POST',
    token,
    body: { emoji },
  });

const reactA = await react(tokenA, helloId, '👍');
check(
  'реакция: первая постановка',
  reactA.status === 200 &&
    reactA.body?.reactions?.length === 1 &&
    reactA.body.reactions[0].emoji === '👍' &&
    reactA.body.reactions[0].count === 1 &&
    reactA.body.reactions[0].me === true,
  JSON.stringify(reactA.body),
);

const reactEvent = await wsB
  .next((m) => m.type === 'message_reactions' && m.payload?.message_id === helloId, 4000)
  .catch(() => null);
check(
  'реакция: подписчик получил message_reactions',
  reactEvent?.payload?.reactions?.length === 1 &&
    reactEvent.payload.reactions[0].count === 1,
  JSON.stringify(reactEvent?.payload),
);

await react(tokenB, helloId, '👍');
const reactB = await react(tokenB, helloId, '❤️');
check(
  'реакция: разные эмодзи группируются отдельно',
  reactB.body?.reactions?.length === 2,
  JSON.stringify(reactB.body?.reactions),
);

const historyReact = await req(`/api/history?chat_id=${dialogId}&limit=20`, { token: tokenA });
const helloWithReactions = Array.isArray(historyReact.body)
  ? historyReact.body.find((m) => m.id === helloId)
  : null;
const thumbs = helloWithReactions?.reactions?.find((r) => r.emoji === '👍');
const heart = helloWithReactions?.reactions?.find((r) => r.emoji === '❤️');
check(
  'реакция: история отдаёт счётчики и флаг «моя»',
  thumbs?.count === 2 && thumbs?.me === true && heart?.count === 1 && heart?.me === false,
  JSON.stringify(helloWithReactions?.reactions),
);

const removeReaction = await react(tokenA, helloId, '👍');
const thumbsAfter = removeReaction.body?.reactions?.find((r) => r.emoji === '👍');
check(
  'реакция: повторный клик снимает её',
  thumbsAfter?.count === 1 && thumbsAfter?.me === false,
  JSON.stringify(removeReaction.body?.reactions),
);

const emptyReaction = await react(tokenA, helloId, '   ');
check('реакция: пустой эмодзи -> 400', emptyReaction.status === 400, `status ${emptyReaction.status}`);

const foreignReaction = await react(tokenC, helloId, '👍');
check('реакция: чужой диалог -> 403', foreignReaction.status === 403, `status ${foreignReaction.status}`);

const deletedReaction = await react(tokenA, replyId, '👍');
check('реакция: удалённое сообщение -> 404', deletedReaction.status === 404, `status ${deletedReaction.status}`);
// ---------- 6. Голосовое сообщение (аудио-вложение) ----------
// Содержимое не проверяется сервером: важен MIME из заголовка.
const voiceBytes = Buffer.from('1a45dfa3a3428681', 'hex');
const voiceUpload = await fetch(
  `${BASE}/api/chats/${dialogId}/media?name=${encodeURIComponent('voice-1.webm')}`,
  {
    method: 'POST',
    headers: { Authorization: `Bearer ${tokenA}`, 'Content-Type': 'audio/webm' },
    body: voiceBytes,
  },
);
const voiceBody = await voiceUpload.json().catch(() => null);
check(
  'голосовое: загрузка audio/webm создаёт сообщение',
  voiceUpload.status === 200 &&
    voiceBody?.is_media === true &&
    voiceBody?.media_type === 'audio/webm',
  `status ${voiceUpload.status}, ${JSON.stringify(voiceBody)}`,
);

const voiceDownload = await fetch(`${BASE}/api/media/${voiceBody?.id}`, {
  headers: { Authorization: `Bearer ${tokenB}` },
});
check(
  'голосовое: скачивание с корректным Content-Type',
  voiceDownload.status === 200 &&
    (voiceDownload.headers.get('content-type') ?? '').startsWith('audio/webm'),
  `status ${voiceDownload.status}, type ${voiceDownload.headers.get('content-type')}`,
);

const badAudio = await fetch(
  `${BASE}/api/chats/${dialogId}/media?name=${encodeURIComponent('x.mid')}`,
  {
    method: 'POST',
    headers: { Authorization: `Bearer ${tokenA}`, 'Content-Type': 'audio/midi' },
    body: Buffer.from([0x4d, 0x54, 0x68, 0x64]),
  },
);
check('голосовое: неподдерживаемый аудио-тип -> 415', badAudio.status === 415, `status ${badAudio.status}`);

wsA.close();
wsB.close();
wsC.close();

console.log(`\n${passed} passed, ${failures.length} failed`);
if (failures.length > 0) {
  console.log(`Провалено: ${failures.join(', ')}`);
  process.exit(1);
}
console.log('MESSAGES E2E OK');
