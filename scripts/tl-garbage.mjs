// Проверка устойчивости MTProto 2.0-сервера к мусорным кадрам.
//
// Скрипт не аутентифицируется: он открывает соединения, присылает заведомо
// некорректные abridged-кадры и проверяет, что процесс не падает (health OK).

import net from 'node:net';
import crypto from 'node:crypto';

const HOST = process.env.KOSTA_HOST || '127.0.0.1';
const PORT = Number(process.env.KOSTA_PORT || 8080);
const HEALTH_URL = process.env.KOSTA_HEALTH_URL || 'http://127.0.0.1:8081/health';
const TIMEOUT_MS = 1500;

function abridgedFrame(payload) {
  if (payload.length % 4 !== 0) {
    throw new Error('abridged payload must be a multiple of 4');
  }
  const words = payload.length / 4;
  let header;
  if (words < 0x7f) {
    header = Buffer.from([words]);
  } else {
    header = Buffer.alloc(4);
    header[0] = 0x7f;
    header.writeUIntLE(words, 1, 3);
  }
  return Buffer.concat([header, payload]);
}

function plainEnvelope(body) {
  const envelope = Buffer.alloc(8 + 8 + 4 + body.length);
  envelope.writeBigInt64LE(0n, 0);
  const msgId = (BigInt(Math.floor(Date.now() / 1000)) << 32n) | 0n;
  envelope.writeBigInt64LE(msgId, 8);
  envelope.writeInt32LE(body.length, 16);
  body.copy(envelope, 20);
  return envelope;
}

function randomPayload(bytes) {
  return crypto.randomBytes(bytes);
}

/** Отправляет init + куски и ждёт, когда сервер закроет соединение. */
function sendRaw(chunks, options = {}) {
  return new Promise((resolve) => {
    const socket = net.connect(PORT, HOST);
    let closed = false;
    let error = null;

    const finish = (reason) => {
      if (closed) return;
      closed = true;
      clearTimeout(timer);
      socket.destroy();
      resolve({ reason, error });
    };

    const timer = setTimeout(() => finish('still-open'), TIMEOUT_MS);

    socket.on('error', (err) => {
      error = err;
      finish('error');
    });
    socket.on('close', () => finish('closed'));
    socket.on('connect', () => {
      socket.write(Buffer.from([options.init ?? 0xef]));
      for (const chunk of chunks) socket.write(chunk);
    });
  });
}

async function healthOk(label) {
  try {
    const response = await fetch(HEALTH_URL);
    if (!response.ok) {
      console.error(`fail ${label}: health ${response.status}`);
      process.exitCode = 1;
      return;
    }
    console.log(`  ok   ${label}: сервер жив (/health 200)`);
  } catch (error) {
    console.error(`fail ${label}: health недоступен: ${error.message}`);
    process.exitCode = 1;
  }
}

async function main() {
  // 1. Случайный мусор в валидных по длине abridged-кадрах.
  {
    const chunks = [];
    for (const size of [4, 8, 64, 512, 2048]) {
      chunks.push(abridgedFrame(randomPayload(size)));
    }
    const result = await sendRaw(chunks);
    console.log(`  ok   случайный мусор: ${result.reason}`);
    await healthOk('случайный мусор');
  }

  // 2. Валидный plaintext-конверт с неизвестным TL-объектом.
  {
    const body = Buffer.concat([
      Buffer.from([0xde, 0xad, 0xbe, 0xef]),
      randomPayload(16),
    ]);
    const result = await sendRaw([abridgedFrame(plainEnvelope(body))]);
    console.log(`  ok   неизвестный TL-объект: ${result.reason}`);
    await healthOk('неизвестный TL-объект');
  }

  // 3. Заголовок кадра с огромной длиной без данных.
  {
    const header = Buffer.from([0x7f, 0xff, 0xff, 0xff]);
    const result = await sendRaw([header]);
    console.log(`  ok   огромная длина кадра: ${result.reason}`);
    await healthOk('огромная длина кадра');
  }

  // 4. Обрезанный кадр: заголовок обещает больше, чем пришло.
  {
    const payload = randomPayload(64);
    const frame = abridgedFrame(payload);
    const result = await sendRaw([frame.subarray(0, 10)]);
    console.log(`  ok   обрезанный кадр: ${result.reason}`);
    await healthOk('обрезанный кадр');
  }

  // 5. Неверный init-байт.
  {
    const result = await sendRaw([], { init: 0xee });
    console.log(`  ok   неверный init-байт: ${result.reason}`);
    await healthOk('неверный init-байт');
  }

  if (process.exitCode) {
    console.error('TL GARBAGE FAILED');
  } else {
    console.log('TL GARBAGE OK');
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});