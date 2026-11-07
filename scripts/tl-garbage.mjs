#!/usr/bin/env node
// Регресс-тест: мусорные и злонамеренные TL-объекты не должны ронять сервер.
// Раньше `Vec<T>` читал count как `i32 as usize`: count = -1 давал
// usize::MAX -> Vec::with_capacity -> паника в потоке, обслуживающем соединение.
//
//   node scripts/tl-garbage.mjs
// Требует запущенный сервер (TCP на 127.0.0.1:8080, web на 127.0.0.1:8081).

import net from 'node:net';

const HOST = process.env.KOSTA_HOST ?? '127.0.0.1';
const PORT = Number(process.env.KOSTA_PORT ?? 8080);
const BASE = process.env.KOSTA_BASE ?? 'http://127.0.0.1:8081';

const CREATE_GROUP_ID = 0xb2c3d4e5;
const HISTORY_RESULT_ID = 0x4d5e6f70;

const u32le = (n) => {
  const b = Buffer.alloc(4);
  b.writeUInt32LE(n >>> 0, 0);
  return b;
};

/** Abridged-рамка вокруг payload. */
function frame(payload) {
  const len = payload.length;
  if (len <= 0x7e) return Buffer.concat([Buffer.from([len]), payload]);
  const header = Buffer.alloc(4);
  header[0] = 0x7f;
  header.writeUIntLE(len, 1, 3);
  return Buffer.concat([header, payload]);
}

// Пустой TL-заголовок строки: len=0 + 3 байта паддинга
const EMPTY_TL_STRING = Buffer.from([0x00, 0x00, 0x00, 0x00]);

const cases = [
  ['CreateGroup, count = -1', Buffer.concat([u32le(CREATE_GROUP_ID), EMPTY_TL_STRING, u32le(-1)])],
  ['CreateGroup, count = i32::MAX', Buffer.concat([u32le(CREATE_GROUP_ID), EMPTY_TL_STRING, u32le(0x7fffffff)])],
  ['HistoryResult, count = i32::MAX', Buffer.concat([u32le(HISTORY_RESULT_ID), u32le(0x7fffffff)])],
  ['ResPQ, длина строки 16 МиБ без данных', Buffer.concat([u32le(0x9a2b3c4d), Buffer.from([0xfe, 0xff, 0xff, 0xff])])],
  ['обрезанный объект', u32le(CREATE_GROUP_ID)],
  ['случайный мусор', Buffer.from([0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03])],
];

const socket = net.connect(PORT, HOST);
let closed = false;
socket.on('close', () => {
  closed = true;
});
socket.on('error', (e) => {
  closed = true;
  console.log(`  socket error (ожидаемо, если сервер закрыл): ${e.code}`);
});

await new Promise((resolve, reject) => {
  socket.once('connect', resolve);
  socket.once('error', reject);
});

for (const [name, payload] of cases) {
  socket.write(frame(payload));
  await new Promise((r) => setTimeout(r, 120));
  console.log(`  отправлено: ${name}`);
}

await new Promise((r) => setTimeout(r, 400));

let ok = true;
if (closed) {
  ok = false;
  console.log('FAIL: сервер закрыл соединение на мусорном вводе');
} else {
  console.log('  ok   соединение не разорвано (парсер вернул ошибку, а не паника)');
}
socket.destroy();

// Сервер всё ещё жив и отвечает
try {
  const res = await fetch(`${BASE}/health`);
  const body = await res.json();
  if (res.status === 200 && body.status === 'ok') {
    console.log('  ok   /health после мусорных пакетов');
  } else {
    ok = false;
    console.log(`FAIL: /health вернул ${res.status}`);
  }
} catch (e) {
  ok = false;
  console.log(`FAIL: сервер не отвечает: ${e.message}`);
}

if (!ok) process.exit(1);
console.log('\nTL GARBAGE OK');
