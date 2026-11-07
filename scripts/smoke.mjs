#!/usr/bin/env node
// Smoke-тест KoSta: web-API + WebSocket.
// Требует запущенный сервер (cargo run -p kosta-server) и Node.js >= 22
// (нужны встроенные fetch и WebSocket).
//
//   node scripts/smoke.mjs
//   node scripts/smoke.mjs --phone=+79990000000
//
// Переменные окружения: KOSTA_BASE (по умолчанию http://127.0.0.1:8081),
// KOSTA_PHONE, KOSTA_PASSWORD (по умолчанию testpass123).

const BASE = process.env.KOSTA_BASE ?? 'http://127.0.0.1:8081';
const WS_URL = BASE.replace(/^http/, 'ws') + '/ws';

const phoneArg = process.argv.find((a) => a.startsWith('--phone='))?.split('=')[1];
const phone = phoneArg ?? process.env.KOSTA_PHONE ?? `+7999${String(Date.now()).slice(-7)}`;
const password = process.env.KOSTA_PASSWORD ?? 'testpass123';
// Номер, которого заведомо нет в базе: по нему проверяем, что аккаунт
// не создаётся без явного register: true.
const unknownPhone = `+7888${String(Date.now()).slice(-7)}`;

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

/**
 * Подключение к /ws с очередью входящих сообщений.
 * next() читает строго по порядку: не подходящие под предикат сообщения
 * отбрасываются, а не заставляют ждать следующего.
 */
function wsSession() {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(WS_URL);
    const queue = [];
    let waiter = null;

    ws.addEventListener('message', (ev) => {
      const msg = JSON.parse(ev.data);
      if (waiter) {
        const pending = waiter;
        waiter = null;
        pending.resolve(msg);
      } else {
        queue.push(msg);
      }
    });
    ws.addEventListener('error', () => reject(new Error('websocket error')));

    ws.addEventListener('open', () => {
      const waitMessage = (timeoutMs) =>
        new Promise((res, rej) => {
          waiter = { resolve: res };
          setTimeout(() => {
            if (waiter) {
              waiter = null;
              rej(new Error('timeout waiting for ws message'));
            }
          }, timeoutMs);
        });

      resolve({
        send: (obj) => ws.send(JSON.stringify(obj)),
        next: async (pred = () => true, timeoutMs = 3000) => {
          const deadline = Date.now() + timeoutMs;
          for (;;) {
            if (queue.length > 0) {
              const msg = queue.shift();
              if (pred(msg)) return msg;
              continue;
            }
            const remaining = deadline - Date.now();
            if (remaining <= 0) throw new Error('timeout waiting for ws message');
            const msg = await waitMessage(remaining);
            if (pred(msg)) return msg;
          }
        },
        close: () => ws.close(),
      });
    });
  });
}

console.log(`KoSta smoke -> ${BASE}, phone ${phone}\n`);

// 1. Перечисление аккаунтов закрыто: неизвестный номер без register -> 401.
//    Сервер не должен ни создавать аккаунт, ни выдавать токен.
const noRegister = await req('/api/login', {
  method: 'POST',
  body: { phone: unknownPhone, password },
});
check(
  'login: неизвестный номер без register -> 401',
  noRegister.status === 401,
  `status ${noRegister.status}`,
);
check('login: без register токен не выдан', !noRegister.body?.token);

// 2. Тот же номер с register: true — явная регистрация проходит.
const registered = await req('/api/login', {
  method: 'POST',
  body: { phone: unknownPhone, password, register: true },
});
check(
  'login: register: true создаёт аккаунт',
  registered.status === 200 && registered.body?.success === true,
  JSON.stringify(registered.body),
);
check(
  'login: регистрация выдаёт токен (64 hex)',
  typeof registered.body?.token === 'string' && /^[0-9a-f]{64}$/.test(registered.body.token),
);

// 3. Основной сценарий. register: true — телефон мог остаться с прошлого прогона,
//    тогда это просто вход.
const login = await req('/api/login', {
  method: 'POST',
  body: { phone, password, register: true },
});
check('login: success', login.body?.success === true, JSON.stringify(login.body));
const token = login.body?.token;
const userId = login.body?.user_id;
check('login: выдан токен (64 hex)', typeof token === 'string' && /^[0-9a-f]{64}$/.test(token));

// 4. Неверный пароль не должен регистрировать второй аккаунт
const badPass = await req('/api/login', {
  method: 'POST',
  body: { phone, password: `${password}-wrong` },
});
check('login: неверный пароль -> 401', badPass.status === 401, `status ${badPass.status}`);

// 5. Валидация входа
const badPhone = await req('/api/login', {
  method: 'POST',
  body: { phone: 'abc', password },
});
check('login: невалидный телефон -> 400', badPhone.status === 400, `status ${badPhone.status}`);

// 6. Без токена и с мусорным токеном доступа нет
const noToken = await req('/api/chats');
check('chats: без токена -> 401', noToken.status === 401, `status ${noToken.status}`);
const garbage = await req('/api/chats', { token: 'deadbeef' });
check('chats: мусорный токен -> 401', garbage.status === 401, `status ${garbage.status}`);
const foreign = await req('/api/history?chat_id=999999&limit=1', { token });
check('history: несуществующий диалог -> 404', foreign.status === 404, `status ${foreign.status}`);

// 7. WebSocket: старый способ (user_id) отклонён, токен принят
const ws1 = await wsSession();
const hello = await ws1.next((m) => m.type === 'connected');
check('ws: подключение', hello.type === 'connected');
ws1.send({ action: 'auth', payload: { user_id: userId } });
const legacy = await ws1.next();
check('ws: авторизация по user_id отклонена', legacy.type === 'auth_failed', legacy.type);
ws1.send({ action: 'auth', payload: { token } });
const auth = await ws1.next();
check('ws: авторизация по токену', auth.type === 'auth_success', auth.type);
ws1.close();

// 8. Чаты пользователя
const chats = await req('/api/chats', { token });
check('chats: 200 и массив', chats.status === 200 && Array.isArray(chats.body), `status ${chats.status}`);

// 9. Полный цикл переписки (если диалог уже создан через example test_client)
if (Array.isArray(chats.body) && chats.body.length > 0) {
  const dialogId = chats.body[0].id;
  const ws2 = await wsSession();
  await ws2.next((m) => m.type === 'connected');
  ws2.send({ action: 'auth', payload: { token } });
  const auth2 = await ws2.next((m) => m.type === 'auth_success' || m.type === 'auth_failed');
  check('ws: повторная авторизация по токену', auth2.type === 'auth_success', auth2.type);

  ws2.send({ action: 'subscribe', payload: { dialog_id: dialogId } });
  const sub = await ws2.next((m) => m.type === 'subscribed' || m.type === 'error');
  check('ws: подписка на диалог', sub.type === 'subscribed', JSON.stringify(sub.payload));

  const text = `smoke ${Date.now()}`;
  ws2.send({ action: 'send_message', payload: { dialog_id: dialogId, text } });
  const incoming = await ws2.next(
    (m) => m.type === 'new_message' && m.payload?.text === text,
  );
  check('ws: сообщение доставлено', Boolean(incoming));

  const hist = await req(`/api/history?chat_id=${dialogId}&limit=10`, { token });
  check(
    'history: сообщение сохранено',
    hist.status === 200 && Array.isArray(hist.body) && hist.body.some((m) => m.text === text),
  );
  ws2.close();
} else {
  console.log('  skip полный цикл: у пользователя нет диалогов');
  console.log(`       создай его: KOSTA_PHONE=${phone} cargo run -p kosta --example test_client`);
}

// 10. Logout отзывает токен и рвёт уже открытые WS-сокеты
const ws3 = await wsSession();
await ws3.next((m) => m.type === 'connected');
ws3.send({ action: 'auth', payload: { token } });
const auth3 = await ws3.next((m) => m.type === 'auth_success' || m.type === 'auth_failed');
check('ws: авторизация перед logout', auth3.type === 'auth_success', auth3.type);

const logout = await req('/api/logout', { method: 'POST', token });
check('logout: 200', logout.status === 200, `status ${logout.status}`);
const kicked = await ws3
  .next((m) => m.type === 'auth_failed', 3000)
  .catch(() => null);
check(
  'ws: logout закрывает соединение',
  kicked !== null && kicked.payload?.error === 'Сессия завершена',
  JSON.stringify(kicked?.payload),
);
ws3.close();

const afterLogout = await req('/api/chats', { token });
check('chats: после logout -> 401', afterLogout.status === 401, `status ${afterLogout.status}`);

console.log(`\n${passed} passed, ${failures.length} failed`);
if (failures.length > 0) {
  console.log(`Провалено: ${failures.join(', ')}`);
  process.exit(1);
}
console.log('SMOKE OK');
