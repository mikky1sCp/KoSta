#!/usr/bin/env node
// E2E поиска по истории сообщений.
//
//   node scripts/search.mjs
//
// Требует запущенный сервер (локально удобно с KOSTA_RATE_LIMIT_DISABLED=1
// и отдельной базой).

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

const search = (token, chatId, query) =>
  req(
    `/api/search?chat_id=${chatId}&q=${encodeURIComponent(query)}`,
    { token },
  );

console.log(`KoSta search e2e -> ${BASE}\n`);

const stamp = String(Date.now()).slice(-7);
const phoneA = `+7992${stamp}`;
const phoneB = `+7991${stamp}`;

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
check('пользователи созданы', Boolean(tokenA && tokenB));

const chat = await req('/api/chats/private', {
  method: 'POST',
  token: tokenA,
  body: { phone: phoneB },
});
const dialogId = chat.body?.chat?.id;
check('диалог создан', Boolean(dialogId));

const wsA = await connectWs(tokenA);
await wsA.wait((m) => m.type === 'auth_success');
wsA.send({ action: 'subscribe', payload: { dialog_id: dialogId } });
await wsA.wait((m) => m.type === 'subscribed');

const texts = [
  `Привет мир ${stamp}`,
  `hello world ${stamp}`,
  `ПРИВЕТ снова ${stamp}`,
  `скидка 100% ${stamp}`,
];

for (const text of texts) {
  wsA.send({ action: 'send_message', payload: { dialog_id: dialogId, text } });
  await wsA.wait((m) => m.type === 'new_message' && m.payload?.text === text);
}

// ---------- Поиск ----------
const cyrillic = await search(tokenA, dialogId, 'привет');
check(
  'поиск по кириллице без учёта регистра',
  cyrillic.status === 200 && cyrillic.body?.length === 2,
  `status ${cyrillic.status}, найдено ${cyrillic.body?.length}`,
);

const ascii = await search(tokenA, dialogId, 'HELLO');
check(
  'поиск по латинице без учёта регистра',
  ascii.body?.length === 1 && ascii.body[0]?.text.includes('hello'),
  JSON.stringify(ascii.body),
);

const percent = await search(tokenA, dialogId, '100%');
check(
  'символ % не превращается в шаблон',
  percent.body?.length === 1 && percent.body[0]?.text.includes('100%'),
  JSON.stringify(percent.body?.length),
);

const partial = await search(tokenA, dialogId, 'мир');
check('поиск по части слова', partial.body?.length === 1, String(partial.body?.length));

const none = await search(tokenA, dialogId, 'нетакоготекста');
check('нет совпадений — пустой массив', Array.isArray(none.body) && none.body.length === 0);

const otherUserSearch = await search(tokenB, dialogId, 'привет');
check(
  'участник диалога тоже может искать',
  otherUserSearch.status === 200 && otherUserSearch.body?.length === 2,
  `status ${otherUserSearch.status}`,
);

const emptyQuery = await search(tokenA, dialogId, '   ');
check('пустой запрос -> 400', emptyQuery.status === 400, `status ${emptyQuery.status}`);

const noToken = await search(undefined, dialogId, 'привет');
check('без токена -> 401', noToken.status === 401, `status ${noToken.status}`);

// Чужой диалог: третий пользователь не должен видеть результаты.
const phoneC = `+7990${stamp}`;
const loginC = await req('/api/login', {
  method: 'POST',
  body: { phone: phoneC, password: PASSWORD, register: true },
});
const foreignSearch = await search(loginC.body?.token, dialogId, 'привет');
check('чужой диалог -> 403', foreignSearch.status === 403, `status ${foreignSearch.status}`);

// Редактирование обновляет индекс поиска.
const firstId = partial.body?.[0]?.id;
const edit = await req(`/api/messages/${firstId}`, {
  method: 'PATCH',
  token: tokenA,
  body: { text: `переименовано ${stamp}` },
});
check('сообщение отредактировано', edit.status === 200, `status ${edit.status}`);

const afterEditOld = await search(tokenA, dialogId, 'мир');
check('старый текст больше не находится', afterEditOld.body?.length === 0, String(afterEditOld.body?.length));

const afterEditNew = await search(tokenA, dialogId, 'переименовано');
check('новый текст находится', afterEditNew.body?.length === 1, String(afterEditNew.body?.length));

// Удалённое сообщение не находится.
const deleteRes = await req(`/api/messages/${firstId}`, { method: 'DELETE', token: tokenA });
check('сообщение удалено', deleteRes.status === 200, `status ${deleteRes.status}`);
const afterDelete = await search(tokenA, dialogId, 'переименовано');
check('удалённое сообщение не находится', afterDelete.body?.length === 0, String(afterDelete.body?.length));

wsA.close();

console.log(`\n${passed} passed, ${failures.length} failed`);
if (failures.length > 0) {
  console.log(`Провалено: ${failures.join(', ')}`);
  process.exit(1);
}
console.log('SEARCH E2E OK');
