#!/usr/bin/env node
// Нагрузочный тест KoSta: N пользователей, WebSocket-соединения, групповой чат.
//
// Что измеряет:
//   * сколько WS-соединений сервер держит и как быстро авторизует;
//   * задержку "отправил → получил эхо своего сообщения" (это запись в БД +
//     рассылка подписчикам, то есть основной путь сообщения);
//   * сколько событий рассылки в секунду переживает сервер (fan-out);
//   * ошибки: отказы авторизации, ошибки WS, потерянные сообщения.
//
// Требует запущенный сервер и Node.js >= 22 (встроенные fetch и WebSocket).
//
//   node scripts/load.mjs --users=20 --duration=15
//   node scripts/load.mjs --users=50 --duration=30 --interval=2000
//
// Для честного теста сервер нужно поднимать с отключёнными лимитами, иначе
// 10 авторизаций в минуту с одного IP обрубят сценарий:
//
//   KOSTA_RATE_LIMIT_DISABLED=1 KOSTA_DB_PATH=loadtest.db cargo run -p kosta-server

const BASE = process.env.KOSTA_BASE ?? 'http://127.0.0.1:8081';
const WS_URL = BASE.replace(/^http/, 'ws') + '/ws';

function arg(name, fallback) {
  const raw = process.argv.find((a) => a.startsWith(`--${name}=`))?.split('=')[1];
  if (raw === undefined) return fallback;
  const num = Number(raw);
  return Number.isFinite(num) ? num : raw;
}

const USERS = Math.max(2, Number(arg('users', 20)));
const DURATION_SEC = Math.max(1, Number(arg('duration', 15)));
const INTERVAL_MS = Math.max(50, Number(arg('interval', 1000)));
const PASSWORD = process.env.KOSTA_PASSWORD ?? 'testpass123';

let passed = 0;
const problems = [];

function note(name, ok, detail = '') {
  if (ok) {
    passed += 1;
    console.log(`  ok   ${name}`);
  } else {
    problems.push(`${name}${detail ? ` — ${detail}` : ''}`);
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

/** WS-соединение с обработчиком всех входящих кадров. */
function openWs({ token, onFrame }) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(WS_URL);
    const timer = setTimeout(() => reject(new Error('ws open timeout')), 15000);

    ws.addEventListener('error', () => {
      clearTimeout(timer);
      reject(new Error('websocket error'));
    });

    ws.addEventListener('message', (ev) => {
      let frame;
      try {
        frame = JSON.parse(ev.data);
      } catch {
        return;
      }
      if (frame.type === 'auth_success') {
        clearTimeout(timer);
        resolve({ send: (obj) => ws.send(JSON.stringify(obj)), close: () => ws.close() });
      }
      onFrame(frame);
    });

    ws.addEventListener('open', () => {
      ws.send(JSON.stringify({ action: 'auth', payload: { token } }));
    });
  });
}

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------- 1. Пользователи ----------
console.log(`KoSta load -> ${BASE}`);
console.log(`users=${USERS} duration=${DURATION_SEC}s interval=${INTERVAL_MS}ms\n`);

const stamp = String(Date.now()).slice(-6);
const started = Date.now();
const accounts = [];
for (let i = 0; i < USERS; i += 1) {
  const phone = `+7999${stamp}${String(i).padStart(3, '0')}`;
  const res = await req('/api/login', {
    method: 'POST',
    body: { phone, password: PASSWORD, register: true },
  });
  if (res.status !== 200 || !res.body?.token) {
    console.log(`  FAIL регистрация пользователя #${i}: ${res.status} ${JSON.stringify(res.body)}`);
    process.exit(1);
  }
  accounts.push({ phone, token: res.body.token, userId: res.body.user_id });
}
const registerMs = Date.now() - started;
console.log(`  ok   ${USERS} аккаунтов за ${registerMs} мс (${Math.round(registerMs / USERS)} мс/аккаунт)`);

// ---------- 2. Групповой чат со всеми ----------
const groupRes = await req('/api/chats/group', {
  method: 'POST',
  token: accounts[0].token,
  body: { title: `load ${stamp}`, phones: accounts.slice(1).map((a) => a.phone) },
});
if (groupRes.status !== 200) {
  console.log(`  FAIL создание группы: ${groupRes.status} ${JSON.stringify(groupRes.body)}`);
  process.exit(1);
}
const groupId = groupRes.body.chat.id;
const members = groupRes.body.participants?.length ?? 0;
note(`группа создана, участников: ${members}`, members === USERS, `ожидали ${USERS}`);

// ---------- 3. WS-соединения ----------
const pending = new Map(); // text -> { t0, resolve }
const latencies = [];
let fanoutEvents = 0;
let wsErrors = 0;
let authFailed = 0;
let unmatched = 0;

function handleFrame(frame) {
  if (frame.type === 'new_message') {
    fanoutEvents += 1;
    const text = frame.payload?.text ?? '';
    const entry = pending.get(text);
    if (entry) {
      pending.delete(text);
      latencies.push(Date.now() - entry.t0);
      entry.resolve();
    }
  } else if (frame.type === 'error') {
    wsErrors += 1;
  } else if (frame.type === 'auth_failed') {
    authFailed += 1;
  }
}

const connStarted = Date.now();
const conns = await Promise.all(
  accounts.map((account) => openWs({ token: account.token, onFrame: handleFrame })),
);
const connectMs = Date.now() - connStarted;
note(
  `${conns.length} WS-соединений авторизованы за ${connectMs} мс`,
  conns.length === USERS && authFailed === 0,
  authFailed ? `auth_failed: ${authFailed}` : '',
);

for (const conn of conns) conn.send({ action: 'subscribe', payload: { dialog_id: groupId } });
await sleep(500);

// ---------- 4. Нагрузка ----------
let seq = 0;
let sent = 0;
let sendFailed = 0;

function sendOne(userIndex) {
  const text = `load-${stamp}-${userIndex}-${(seq += 1)}`;
  return new Promise((resolve) => {
    pending.set(text, {
      t0: Date.now(),
      resolve: () => {
        sent += 1;
        resolve();
      },
    });
    const ok = (() => {
      try {
        conns[userIndex].send({ action: 'send_message', payload: { dialog_id: groupId, text } });
        return true;
      } catch {
        return false;
      }
    })();
    if (!ok) {
      pending.delete(text);
      sendFailed += 1;
      resolve();
      return;
    }
    // Сообщение могло не вернуться — не держим память вечно.
    setTimeout(() => {
      if (pending.delete(text)) {
        unmatched += 1;
        resolve();
      }
    }, 20000);
  });
}

const loadStarted = Date.now();
const deadline = loadStarted + DURATION_SEC * 1000;
let tick = 0;

while (Date.now() < deadline) {
  const wave = [];
  for (let i = 0; i < USERS; i += 1) wave.push(sendOne(i));
  await Promise.all(wave);
  tick += 1;
  const elapsedSec = (Date.now() - loadStarted) / 1000;
  if (tick % 5 === 0 && elapsedSec > 0) {
    console.log(
      `  … ${elapsedSec.toFixed(0)}s: отправлено ${sent}, событий рассылки ${fanoutEvents}, ` +
        `средняя задержка ${Math.round(latencies.reduce((a, b) => a + b, 0) / Math.max(1, latencies.length))} мс`,
    );
  }
  await sleep(INTERVAL_MS);
}

const loadMs = Date.now() - loadStarted;

// ---------- 5. Итоги ----------
function percentile(values, p) {
  if (values.length === 0) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const idx = Math.min(sorted.length - 1, Math.floor((p / 100) * sorted.length));
  return sorted[idx];
}

const totalSent = sent;
const throughput = totalSent / (loadMs / 1000);
const avg = latencies.length
  ? latencies.reduce((a, b) => a + b, 0) / latencies.length
  : 0;

console.log('\n=== Результат ===');
console.log(`  отправлено сообщений: ${totalSent} (${throughput.toFixed(1)}/с)`);
console.log(`  событий рассылки:      ${fanoutEvents} (${(fanoutEvents / (loadMs / 1000)).toFixed(0)}/с)`);
console.log(`  задержка эха:          avg ${Math.round(avg)} мс, p50 ${percentile(latencies, 50)} мс, ` +
  `p95 ${percentile(latencies, 95)} мс, p99 ${percentile(latencies, 99)} мс, max ${Math.max(0, ...latencies)} мс`);
console.log(`  ошибок WS: ${wsErrors}, потеряно: ${unmatched}, не отправлено: ${sendFailed}, auth_failed: ${authFailed}`);

const expected = totalSent * USERS; // каждое сообщение уходит всем подписчикам
console.log(`  fan-out: получено ${fanoutEvents} из ~${expected} ожидаемых ` +
  `(${expected ? Math.round((fanoutEvents / expected) * 100) : 0}%)`);

const lostRatio = totalSent ? unmatched / totalSent : 1;
note(`нет потерянных сообщений (${unmatched})`, lostRatio < 0.01, `${(lostRatio * 100).toFixed(1)}% потеряно`);
note(`нет ошибок WS (${wsErrors})`, wsErrors === 0);
note(`p95 задержки < 1000 мс (${percentile(latencies, 95)} мс)`, percentile(latencies, 95) < 1000);

for (const conn of conns) conn.close();

console.log(`\n${passed} passed, ${problems.length} failed`);
if (problems.length > 0) {
  console.log(`Проблемы: ${problems.join('; ')}`);
  process.exit(1);
}
console.log('LOAD OK');
