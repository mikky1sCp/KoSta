#!/usr/bin/env node
// E2E профиля: отображаемое имя, аватар, влияние имени на заголовок чата.
//
//   node scripts/profile.mjs
//
// Требует запущенный сервер (для локального прогона удобно с
// KOSTA_RATE_LIMIT_DISABLED=1 и отдельной базой).

const BASE = process.env.KOSTA_BASE ?? 'http://127.0.0.1:8081';
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

// Минимальный валидный PNG 1x1.
const PNG = Buffer.from(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==',
  'base64',
);

console.log(`KoSta profile e2e -> ${BASE}\n`);

const stamp = String(Date.now()).slice(-7);
const phoneA = `+7994${stamp}`;
const phoneB = `+7993${stamp}`;

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
check('пользователи созданы', Boolean(tokenA && tokenB));

const initialMe = await req('/api/me', { token: tokenA });
check(
  'новый профиль: имени нет, аватара нет',
  initialMe.status === 200 &&
    initialMe.body?.display_name === null &&
    initialMe.body?.has_avatar === false,
  JSON.stringify(initialMe.body),
);

// ---------- Имя ----------
const displayName = `Аня ${stamp.slice(-3)}`;
const renamed = await req('/api/me', {
  method: 'PATCH',
  token: tokenA,
  body: { display_name: displayName },
});
check(
  'PATCH /api/me сохраняет имя',
  renamed.status === 200 && renamed.body?.display_name === displayName,
  JSON.stringify(renamed.body),
);

const meAfter = await req('/api/me', { token: tokenA });
check('GET /api/me отдаёт имя', meAfter.body?.display_name === displayName);

// ---------- Имя в заголовке личного чата ----------
const chat = await req('/api/chats/private', {
  method: 'POST',
  token: tokenB,
  body: { phone: phoneA },
});
check(
  'заголовок личного чата — имя, а не телефон',
  chat.body?.chat?.title === displayName,
  JSON.stringify(chat.body?.chat?.title),
);
const participants = chat.body?.participants ?? [];
const meParticipant = participants.find((p) => p.user_id === userA);
check(
  'участники отдают display_name',
  meParticipant?.display_name === displayName,
  JSON.stringify(meParticipant),
);

// ---------- Аватар ----------
const upload = await fetch(`${BASE}/api/me/avatar`, {
  method: 'POST',
  headers: { Authorization: `Bearer ${tokenA}`, 'Content-Type': 'image/png' },
  body: PNG,
});
check('POST /api/me/avatar -> 200', upload.status === 200, `status ${upload.status}`);

const meWithAvatar = await req('/api/me', { token: tokenA });
check('has_avatar становится true', meWithAvatar.body?.has_avatar === true);

const avatarRes = await fetch(`${BASE}/api/avatars/${userA}`);
const avatarBytes = Buffer.from(await avatarRes.arrayBuffer());
check(
  'GET /api/avatars/{id} отдаёт картинку',
  avatarRes.status === 200 &&
    (avatarRes.headers.get('content-type') ?? '').startsWith('image/png') &&
    avatarBytes.equals(PNG),
  `status ${avatarRes.status}, bytes ${avatarBytes.length}`,
);

const missingAvatar = await fetch(`${BASE}/api/avatars/${loginB.body?.user_id}`);
check('без аватара -> 404', missingAvatar.status === 404, `status ${missingAvatar.status}`);

const badType = await fetch(`${BASE}/api/me/avatar`, {
  method: 'POST',
  headers: { Authorization: `Bearer ${tokenA}`, 'Content-Type': 'text/html' },
  body: Buffer.from('<b>x</b>'),
});
check('неподдерживаемый тип -> 415', badType.status === 415, `status ${badType.status}`);

// Тип объявляет клиент, но сервер сверяет его с сигнатурой: SVG под видом
// PNG не должен сохраниться как аватар.
const spoofedAvatar = await fetch(`${BASE}/api/me/avatar`, {
  method: 'POST',
  headers: { Authorization: `Bearer ${tokenA}`, 'Content-Type': 'image/png' },
  body: Buffer.from('<svg xmlns="http://www.w3.org/2000/svg"></svg>'),
});
check(
  'содержимое не совпадает с типом картинки -> 415',
  spoofedAvatar.status === 415,
  `status ${spoofedAvatar.status}`,
);

const emptyAvatar = await fetch(`${BASE}/api/me/avatar`, {
  method: 'POST',
  headers: { Authorization: `Bearer ${tokenA}`, 'Content-Type': 'image/png' },
  body: Buffer.alloc(0),
});
check('пустой файл -> 400', emptyAvatar.status === 400, `status ${emptyAvatar.status}`);

const noTokenRename = await req('/api/me', {
  method: 'PATCH',
  body: { display_name: 'hacked' },
});
check('правка профиля без токена -> 401', noTokenRename.status === 401, `status ${noTokenRename.status}`);

// ---------- Сброс имени ----------
const cleared = await req('/api/me', {
  method: 'PATCH',
  token: tokenA,
  body: { display_name: '   ' },
});
check('пустое имя сбрасывается', cleared.body?.display_name === null, JSON.stringify(cleared.body));

const chatAfterClear = await req(`/api/chats/${chat.body?.chat?.id}`, { token: tokenB });
check(
  'без имени заголовок снова телефон',
  chatAfterClear.body?.chat?.title === phoneA,
  JSON.stringify(chatAfterClear.body?.chat?.title),
);

const longName = await req('/api/me', {
  method: 'PATCH',
  token: tokenA,
  body: { display_name: 'x'.repeat(65) },
});
check('слишком длинное имя -> 400', longName.status === 400, `status ${longName.status}`);

console.log(`\n${passed} passed, ${failures.length} failed`);
if (failures.length > 0) {
  console.log(`Провалено: ${failures.join(', ')}`);
  process.exit(1);
}
console.log('PROFILE E2E OK');
