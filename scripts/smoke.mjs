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

// 10. Новые API для UI: профиль, lookup, личные чаты, группы, read, typing
const secondPhone = `+7777${String(Date.now()).slice(-7)}`;
const thirdPhone = `+7666${String(Date.now()).slice(-7)}`;
const login2 = await req('/api/login', {
  method: 'POST',
  body: { phone: secondPhone, password, register: true },
});
const token2 = login2.body?.token;
const userId2 = login2.body?.user_id;
check('ui-api: второй пользователь создан', login2.status === 200 && Boolean(token2));

const login3 = await req('/api/login', {
  method: 'POST',
  body: { phone: thirdPhone, password, register: true },
});
const token3 = login3.body?.token;
const userId3 = login3.body?.user_id;
check('ui-api: третий пользователь создан', login3.status === 200 && Boolean(userId3));

const me = await req('/api/me', { token });
check('ui-api: /api/me отдаёт текущего пользователя', me.status === 200 && me.body?.phone === phone);

const lookup = await req(`/api/users/lookup?phone=${encodeURIComponent(secondPhone)}`, { token });
check(
  'ui-api: lookup находит пользователя',
  lookup.status === 200 && lookup.body?.user_id === userId2,
  `status ${lookup.status}`,
);
const lookupMissing = await req('/api/users/lookup?phone=%2B70000000000', { token });
check('ui-api: lookup неизвестного номера -> 404', lookupMissing.status === 404, `status ${lookupMissing.status}`);

const privateChat = await req('/api/chats/private', {
  method: 'POST',
  token,
  body: { phone: secondPhone },
});
const privateId = privateChat.body?.chat?.id;
check(
  'ui-api: личный чат создан',
  privateChat.status === 200 &&
    privateChat.body?.chat?.is_group === false &&
    privateChat.body?.participants?.length === 2,
  JSON.stringify(privateChat.body),
);
const selfChat = await req('/api/chats/private', { method: 'POST', token, body: { phone } });
check('ui-api: личный чат с собой -> 400', selfChat.status === 400, `status ${selfChat.status}`);

// Подписчик первого пользователя нужен для typing/messages_read.
const wsA = await wsSession();
await wsA.next((m) => m.type === 'connected');
wsA.send({ action: 'auth', payload: { token } });
await wsA.next((m) => m.type === 'auth_success');
wsA.send({ action: 'subscribe', payload: { dialog_id: privateId } });
await wsA.next((m) => m.type === 'subscribed');

const wsB = await wsSession();
await wsB.next((m) => m.type === 'connected');
wsB.send({ action: 'auth', payload: { token: token2 } });
await wsB.next((m) => m.type === 'auth_success');
wsB.send({ action: 'subscribe', payload: { dialog_id: privateId } });
await wsB.next((m) => m.type === 'subscribed');

const fromFirst = `private from first ${Date.now()}`;
wsA.send({ action: 'send_message', payload: { dialog_id: privateId, text: fromFirst } });
await wsA.next((m) => m.type === 'new_message' && m.payload?.text === fromFirst);

const chatsForSecond = await req('/api/chats', { token: token2 });
const unreadChat = chatsForSecond.body?.find((c) => c.id === privateId);
check(
  'ui-api: unread_count растёт у получателя',
  chatsForSecond.status === 200 && (unreadChat?.unread_count ?? 0) >= 1,
  JSON.stringify(unreadChat),
);

const readResult = await req(`/api/chats/${privateId}/read`, { method: 'POST', token: token2 });
check(
  'ui-api: read помечает входящие',
  readResult.status === 200 && (readResult.body?.marked ?? 0) >= 1,
  JSON.stringify(readResult.body),
);
const readEvent = await wsA
  .next((m) => m.type === 'messages_read' && m.payload?.dialog_id === privateId, 3000)
  .catch(() => null);
check(
  'ws: messages_read доставлен подписчику',
  readEvent?.payload?.reader_id === userId2,
  JSON.stringify(readEvent?.payload),
);
const chatsAfterRead = await req('/api/chats', { token: token2 });
const unreadAfter = chatsAfterRead.body?.find((c) => c.id === privateId);
check('ui-api: unread_count сброшен', (unreadAfter?.unread_count ?? 1) === 0);

wsB.send({ action: 'typing', payload: { dialog_id: privateId, typing: true } });
const typingEvent = await wsA
  .next((m) => m.type === 'typing' && m.payload?.dialog_id === privateId, 3000)
  .catch(() => null);
check(
  'ws: typing доставлен собеседнику',
  typingEvent?.payload?.user_id === userId2 && typingEvent?.payload?.typing === true,
  JSON.stringify(typingEvent?.payload),
);

// Медиа: загрузка, broadcast, скачивание, проверка прав и MIME.
const pngBytes = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
  'base64',
);
const uploadUrl =
  `${BASE}/api/chats/${privateId}/media?name=${encodeURIComponent('pixel.png')}` +
  `&caption=${encodeURIComponent('media caption')}`;
const uploadRes = await fetch(uploadUrl, {
  method: 'POST',
  headers: { Authorization: `Bearer ${token2}`, 'Content-Type': 'image/png' },
  body: pngBytes,
});
const uploadBody = await uploadRes.json().catch(() => null);
check(
  'media: загрузка создаёт media-сообщение',
  uploadRes.status === 200 &&
    uploadBody?.is_media === true &&
    uploadBody?.media_type === 'image/png' &&
    uploadBody?.media_size === pngBytes.length &&
    uploadBody?.media_name === 'pixel.png',
  JSON.stringify(uploadBody),
);

const mediaBroadcast = await wsA
  .next((m) => m.type === 'new_message' && m.payload?.id === uploadBody?.id, 3000)
  .catch(() => null);
check(
  'media: new_message broadcast подписчику',
  mediaBroadcast?.payload?.is_media === true && mediaBroadcast?.payload?.sender_id === userId2,
  JSON.stringify(mediaBroadcast?.payload),
);

const downloadRes = await fetch(`${BASE}/api/media/${uploadBody?.id}`, {
  headers: { Authorization: `Bearer ${token}` },
});
const downloadBytes = Buffer.from(await downloadRes.arrayBuffer());
check(
  'media: авторизованное скачивание',
  downloadRes.status === 200 &&
    (downloadRes.headers.get('content-type') ?? '').startsWith('image/png') &&
    downloadBytes.equals(pngBytes),
  `status ${downloadRes.status}, bytes ${downloadBytes.length}`,
);

const foreignMedia = await fetch(`${BASE}/api/media/${uploadBody?.id}`, {
  headers: { Authorization: `Bearer ${token3}` },
});
check('media: чужой участник не скачает файл -> 403', foreignMedia.status === 403, `status ${foreignMedia.status}`);

const badMime = await fetch(
  `${BASE}/api/chats/${privateId}/media?name=${encodeURIComponent('x.html')}`,
  {
    method: 'POST',
    headers: { Authorization: `Bearer ${token2}`, 'Content-Type': 'text/html' },
    body: Buffer.from('<b>x</b>'),
  },
);
check('media: запрещённый MIME -> 415', badMime.status === 415, `status ${badMime.status}`);

const emptyUpload = await fetch(
  `${BASE}/api/chats/${privateId}/media?name=${encodeURIComponent('empty.png')}`,
  {
    method: 'POST',
    headers: { Authorization: `Bearer ${token2}`, 'Content-Type': 'image/png' },
    body: Buffer.alloc(0),
  },
);
check('media: пустой файл -> 400', emptyUpload.status === 400, `status ${emptyUpload.status}`);

const mediaHistory = await req(`/api/history?chat_id=${privateId}&limit=10`, { token });
const mediaInHistory = Array.isArray(mediaHistory.body)
  ? mediaHistory.body.find((m) => m.id === uploadBody?.id)
  : null;
check(
  'media: сообщение в истории с media-полями',
  mediaInHistory?.is_media === true &&
    mediaInHistory?.media_name === 'pixel.png' &&
    mediaInHistory?.text === 'media caption',
  JSON.stringify(mediaInHistory),
);

const group = await req('/api/chats/group', {
  method: 'POST',
  token,
  body: { title: 'Smoke group', phones: [secondPhone] },
});
const groupId = group.body?.chat?.id;
check(
  'ui-api: группа создана, создатель админ',
  group.status === 200 &&
    group.body?.chat?.is_group === true &&
    group.body?.is_admin === true &&
    group.body?.participants?.length === 2,
  JSON.stringify(group.body),
);

const groupForMember = await req(`/api/chats/${groupId}`, { token: token2 });
check(
  'ui-api: участник видит детали группы',
  groupForMember.status === 200 && groupForMember.body?.is_admin === false,
  `status ${groupForMember.status}`,
);

const addThird = await req(`/api/chats/${groupId}/participants`, {
  method: 'POST',
  token,
  body: { phone: thirdPhone },
});
check(
  'ui-api: админ добавил участника',
  addThird.status === 200 &&
    addThird.body?.participants?.some((p) => p.user_id === userId3),
  JSON.stringify(addThird.body),
);
const addForbidden = await req(`/api/chats/${groupId}/participants`, {
  method: 'POST',
  token: token2,
  body: { phone: '+70000000000' },
});
check('ui-api: не админ не может добавлять -> 403', addForbidden.status === 403, `status ${addForbidden.status}`);

const rename = await req(`/api/chats/${groupId}`, {
  method: 'PATCH',
  token,
  body: { title: 'Smoke group 2' },
});
check(
  'ui-api: админ переименовал группу',
  rename.status === 200 && rename.body?.chat?.title === 'Smoke group 2',
  JSON.stringify(rename.body?.chat),
);
const renameForbidden = await req(`/api/chats/${groupId}`, {
  method: 'PATCH',
  token: token2,
  body: { title: 'hacked' },
});
check('ui-api: не админ не может переименовать -> 403', renameForbidden.status === 403, `status ${renameForbidden.status}`);

const removeThird = await req(`/api/chats/${groupId}/participants/${userId3}`, {
  method: 'DELETE',
  token,
});
check(
  'ui-api: админ удалил участника',
  removeThird.status === 200 && removeThird.body?.deleted === false,
  JSON.stringify(removeThird.body),
);
const leaveSecond = await req(`/api/chats/${groupId}/participants/${userId2}`, {
  method: 'DELETE',
  token: token2,
});
check(
  'ui-api: участник может выйти из группы',
  leaveSecond.status === 200 && leaveSecond.body?.deleted === false,
  `status ${leaveSecond.status}`,
);
const leaveAdmin = await req(`/api/chats/${groupId}/participants/${userId}`, {
  method: 'DELETE',
  token,
});
check(
  'ui-api: последний участник удаляет диалог',
  leaveAdmin.status === 200 && leaveAdmin.body?.deleted === true,
  JSON.stringify(leaveAdmin.body),
);

wsA.close();
wsB.close();

// 11. Logout отзывает токен и рвёт уже открытые WS-сокеты
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
