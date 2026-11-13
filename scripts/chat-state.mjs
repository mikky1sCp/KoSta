#!/usr/bin/env node
// E2E личного состояния чатов: закрепление, мьют, архив, «непрочитано» и
// удаление чата из списка.
//
//   node scripts/chat-state.mjs
//
// Требует запущенный сервер (локально удобно с KOSTA_RATE_LIMIT_DISABLED=1).

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

function connectWs(token) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(WS_URL);
    const queue = [];
    ws.addEventListener('error', () => reject(new Error('websocket error')));
    ws.addEventListener('message', (ev) => {
      try {
        queue.push(JSON.parse(ev.data));
      } catch {
        /* ignore */
      }
    });
    ws.addEventListener('open', () => {
      ws.send(JSON.stringify({ action: 'auth', payload: { token } }));
      const wait = (pred, timeoutMs = 5000) =>
        new Promise((res, rej) => {
          const deadline = Date.now() + timeoutMs;
          const tick = setInterval(() => {
            const index = queue.findIndex(pred);
            if (index !== -1) {
              const [msg] = queue.splice(index, 1);
              clearInterval(tick);
              res(msg);
              return;
            }
            if (Date.now() > deadline) {
              clearInterval(tick);
              rej(new Error('timeout waiting for ws message'));
            }
          }, 20);
        });
      resolve({
        wait,
        send: (obj) => ws.send(JSON.stringify(obj)),
        close: () => ws.close(),
      });
    });
  });
}

const chatById = async (token, chatId) => {
  const list = await req('/api/chats', { token });
  return (list.body ?? []).find((chat) => chat.id === chatId) ?? null;
};

console.log(`KoSta chat state e2e -> ${BASE}\n`);

const stamp = String(Date.now()).slice(-7);
const phoneA = `+7985${stamp}`;
const phoneB = `+7984${stamp}`;
const phoneC = `+7983${stamp}`;

const loginA = await req('/api/login', {
  method: 'POST',
  body: { phone: phoneA, password: PASSWORD, register: true },
});
const loginB = await req('/api/login', {
  method: 'POST',
  body: { phone: phoneB, password: PASSWORD, register: true },
});
const loginC = await req('/api/login', {
  method: 'POST',
  body: { phone: phoneC, password: PASSWORD, register: true },
});
const tokenA = loginA.body?.token;
const tokenB = loginB.body?.token;
const tokenC = loginC.body?.token;
check('пользователи созданы', Boolean(tokenA && tokenB && tokenC));

const chat = await req('/api/chats/private', {
  method: 'POST',
  token: tokenA,
  body: { phone: phoneB },
});
const dialogId = chat.body?.chat?.id;
check('диалог создан', Boolean(dialogId));

// ---------- Закрепление ----------
const pinned = await req(`/api/chats/${dialogId}/state`, {
  method: 'PATCH',
  token: tokenA,
  body: { pinned: true },
});
check(
  'чат закреплён',
  pinned.status === 200 && pinned.body?.chat?.pinned === true,
  `status ${pinned.status}`,
);
check('закрепление видно в списке', (await chatById(tokenA, dialogId))?.pinned === true);
check(
  'закрепление не влияет на собеседника',
  (await chatById(tokenB, dialogId))?.pinned === false,
);

const unpinned = await req(`/api/chats/${dialogId}/state`, {
  method: 'PATCH',
  token: tokenA,
  body: { pinned: false },
});
check('чат откреплён', unpinned.body?.chat?.pinned === false);

// ---------- Мьют ----------
const now = Math.floor(Date.now() / 1000);
const muted1h = await req(`/api/chats/${dialogId}/state`, {
  method: 'PATCH',
  token: tokenA,
  body: { muted: true, mute_duration: '1h' },
});
const mutedUntil = muted1h.body?.chat?.muted_until;
check(
  'мьют на 1 час',
  muted1h.body?.chat?.muted === true &&
    typeof mutedUntil === 'number' &&
    mutedUntil > now + 3500 &&
    mutedUntil <= now + 3700,
  JSON.stringify({ muted: muted1h.body?.chat?.muted, mutedUntil, now }),
);
check(
  'мьют не влияет на собеседника',
  (await chatById(tokenB, dialogId))?.muted === false,
);

const mutedForever = await req(`/api/chats/${dialogId}/state`, {
  method: 'PATCH',
  token: tokenA,
  body: { muted: true, mute_duration: 'forever' },
});
check(
  'мьют навсегда',
  mutedForever.body?.chat?.muted === true && mutedForever.body?.chat?.muted_until === 0,
  JSON.stringify(mutedForever.body?.chat),
);

const unmuted = await req(`/api/chats/${dialogId}/state`, {
  method: 'PATCH',
  token: tokenA,
  body: { muted: false },
});
check(
  'мьют снят',
  unmuted.body?.chat?.muted === false && unmuted.body?.chat?.muted_until == null,
);

const badDuration = await req(`/api/chats/${dialogId}/state`, {
  method: 'PATCH',
  token: tokenA,
  body: { muted: true, mute_duration: '2h' },
});
check('неизвестный срок мьюта -> 400', badDuration.status === 400, `status ${badDuration.status}`);

// ---------- Архив ----------
const archived = await req(`/api/chats/${dialogId}/state`, {
  method: 'PATCH',
  token: tokenA,
  body: { archived: true },
});
check('чат в архиве', archived.body?.chat?.archived === true);
const archivedChat = await chatById(tokenA, dialogId);
check(
  'архивный чат остаётся в списке (для секции «Архив»)',
  archivedChat !== null && archivedChat.archived === true,
);
check('архив не влияет на собеседника', (await chatById(tokenB, dialogId))?.archived === false);

const unarchived = await req(`/api/chats/${dialogId}/state`, {
  method: 'PATCH',
  token: tokenA,
  body: { archived: false },
});
check('чат возвращён из архива', unarchived.body?.chat?.archived === false);

// ---------- Ручная пометка «непрочитано» ----------
const marked = await req(`/api/chats/${dialogId}/state`, {
  method: 'PATCH',
  token: tokenA,
  body: { marked_unread: true },
});
check('чат помечен непрочитанным', marked.body?.chat?.marked_unread === true);

const read = await req(`/api/chats/${dialogId}/read`, { method: 'POST', token: tokenA });
check('пометка снята прочтением', read.status === 200);
check('marked_unread сброшен', (await chatById(tokenA, dialogId))?.marked_unread === false);

// ---------- Чужой чат ----------
const foreign = await req(`/api/chats/${dialogId}/state`, {
  method: 'PATCH',
  token: tokenC,
  body: { pinned: true },
});
check('чужой чат -> 403', foreign.status === 403, `status ${foreign.status}`);

// ---------- Удаление личного чата ----------
const del = await req(`/api/chats/${dialogId}`, { method: 'DELETE', token: tokenA });
check(
  'личный чат удалён у пользователя',
  del.status === 200 && del.body?.deleted === false,
  `status ${del.status}`,
);
check('удалённый чат пропал из списка', (await chatById(tokenA, dialogId)) === null);
check(
  'у собеседника чат остался',
  (await chatById(tokenB, dialogId)) !== null,
);

// Новое сообщение от собеседника возвращает чат в список.
const wsB = await connectWs(tokenB);
await wsB.wait((m) => m.type === 'auth_success');
const text = `вернулся ${stamp}`;
wsB.send({ action: 'subscribe', payload: { dialog_id: dialogId } });
await wsB.wait((m) => m.type === 'subscribed');
wsB.send({ action: 'send_message', payload: { dialog_id: dialogId, text } });
await wsB.wait((m) => m.type === 'new_message' && m.payload?.text === text);
const returned = await chatById(tokenA, dialogId);
check('новое сообщение возвращает удалённый чат', returned !== null, String(returned));
wsB.close();

// ---------- Удаление группы (выход) ----------
const group = await req('/api/chats/group', {
  method: 'POST',
  token: tokenA,
  body: { title: `Группа ${stamp}`, phones: [phoneB] },
});
const groupId = group.body?.chat?.id;
check('группа создана', Boolean(groupId));

const leave = await req(`/api/chats/${groupId}`, { method: 'DELETE', token: tokenA });
check(
  'участник покинул группу',
  leave.status === 200 && leave.body?.deleted === false,
  `status ${leave.status}`,
);
check('группа пропала у вышедшего', (await chatById(tokenA, groupId)) === null);
check('у второго участника группа осталась', (await chatById(tokenB, groupId)) !== null);

const lastLeave = await req(`/api/chats/${groupId}`, { method: 'DELETE', token: tokenB });
check(
  'последний участник удаляет группу',
  lastLeave.status === 200 && lastLeave.body?.deleted === true,
  `status ${lastLeave.status}`,
);

// ---------- Валидация ----------
const noToken = await req(`/api/chats/${dialogId}/state`, {
  method: 'PATCH',
  body: { pinned: true },
});
check('без токена -> 401', noToken.status === 401, `status ${noToken.status}`);

const unknownChat = await req('/api/chats/999999/state', {
  method: 'PATCH',
  token: tokenA,
  body: { pinned: true },
});
check('несуществующий чат -> 404', unknownChat.status === 404, `status ${unknownChat.status}`);

console.log(`\n${passed} passed, ${failures.length} failed`);
if (failures.length > 0) {
  console.log(`Провалено: ${failures.join(', ')}`);
  process.exit(1);
}
console.log('CHAT STATE E2E OK');
