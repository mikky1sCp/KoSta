#!/usr/bin/env node
// Замер пропускной способности логина/регистрации: bcrypt дорогой и в этом
// проекте вызывается синхронно внутри async-хендлера, поэтому именно вход
// становится потолком при массовом наплыве пользователей.
//
//   node scripts/login-bench.mjs --count=32
//
// Создаёт --count аккаунтов (последовательно), затем выполняет --count входов
// ОДНОВРЕМЕННО и показывает фактический потолок входов в секунду.

const BASE = process.env.KOSTA_BASE ?? 'http://127.0.0.1:8081';

function arg(name, fallback) {
  const raw = process.argv.find((a) => a.startsWith(`--${name}=`))?.split('=')[1];
  return raw === undefined ? fallback : Number(raw);
}

const COUNT = Math.max(2, arg('count', 32));
const PASSWORD = process.env.KOSTA_PASSWORD ?? 'testpass123';
const stamp = String(Date.now()).slice(-6);

async function login(phone) {
  const res = await fetch(`${BASE}/api/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ phone, password: PASSWORD }),
    signal: AbortSignal.timeout(60000),
  });
  return res.status;
}

console.log(`Логин-бенчмарк KoSta -> ${BASE}, count=${COUNT}\n`);

const phones = Array.from({ length: COUNT }, (_, i) => `+7998${stamp}${String(i).padStart(3, '0')}`);

// 1. Создание аккаунтов последовательно (bcrypt при регистрации).
let t0 = Date.now();
for (const phone of phones) {
  const res = await fetch(`${BASE}/api/login`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ phone, password: PASSWORD, register: true }),
    signal: AbortSignal.timeout(60000),
  });
  if (res.status !== 200) {
    console.log(`  FAIL регистрация ${phone}: ${res.status}`);
    process.exit(1);
  }
}
const seqMs = Date.now() - t0;
console.log(`Регистрация последовательно: ${seqMs} мс на ${COUNT} = ${Math.round(seqMs / COUNT)} мс/аккаунт`);

// 2. Входы одновременно — измеряем реальный параллелизм сервера.
t0 = Date.now();
const statuses = await Promise.all(phones.map((p) => login(p)));
const parMs = Date.now() - t0;

const okCount = statuses.filter((s) => s === 200).length;
const perSec = okCount / (parMs / 1000);
const avgMs = parMs / COUNT;

console.log(`Параллельные входы: ${okCount}/${COUNT} успешно за ${parMs} мс`);
console.log(`  средняя задержка входа: ${Math.round(avgMs)} мс`);
console.log(`  пропускная способность: ${perSec.toFixed(1)} входов/с`);
console.log('');
console.log('Справка: 1 000 000 входов за час = 278 входов/с; за сутки = 11,6 входов/с.');
if (perSec >= 278) {
  console.log('  ВЫВОД: пропускной способности хватает на 1 млн входов в час.');
} else if (perSec >= 12) {
  console.log(`  ВЫВОД: хватает примерно на ${Math.round(perSec * 86400 / 1000000 * 10) / 10} млн входов в сутки, но не в час.`);
} else {
  console.log('  ВЫВОД: недостаточно даже для суточного распределения — нужна оптимизация входа.');
}
