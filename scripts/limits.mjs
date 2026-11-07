// Проверка лимитов MTProto 2.0-сервера.
//
// 1. Rate limit рукопожатий: сервер ограничивает req_pq/DH-операции по IP
//    (30/мин). Скрипт открывает одно соединение и шлёт req_pq до закрытия.
// 2. Опционально idle timeout: KOSTA_EXPECT_IDLE_TIMEOUT=1 + короткий
//    idle_timeout_secs в конфиге — сервер должен закрыть простаивающее
//    соединение.

import net from 'node:net';
import crypto from 'node:crypto';

const HOST = process.env.KOSTA_HOST || '127.0.0.1';
const PORT = Number(process.env.KOSTA_PORT || 8080);
const HEALTH_URL = process.env.KOSTA_HEALTH_URL || 'http://127.0.0.1:8081/health';
const EXPECT_IDLE_TIMEOUT = process.env.KOSTA_EXPECT_IDLE_TIMEOUT === '1';
const REQ_PQ_ID = 0x60469778;

function abridgedFrame(payload) {
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

function reqPqFrame(counter) {
  const body = Buffer.alloc(4 + 16);
  body.writeInt32LE(REQ_PQ_ID, 0);
  crypto.randomBytes(16).copy(body, 4);

  const msgId = (BigInt(Math.floor(Date.now() / 1000)) << 32n) | (BigInt(counter) << 2n);
  const envelope = Buffer.alloc(8 + 8 + 4 + body.length);
  envelope.writeBigInt64LE(0n, 0);
  envelope.writeBigInt64LE(msgId, 8);
  envelope.writeInt32LE(body.length, 16);
  body.copy(envelope, 20);
  return abridgedFrame(envelope);
}

function handshakeUntilClosed(maxRequests = 60) {
  return new Promise((resolve, reject) => {
    const socket = net.connect(PORT, HOST);
    let counter = 0;
    let closed = false;

    const timer = setTimeout(() => {
      if (!closed) {
        closed = true;
        socket.destroy();
        resolve({ closed: false, counter });
      }
    }, 5000);

    socket.on('error', (error) => {
      if (!closed) {
        closed = true;
        clearTimeout(timer);
        resolve({ closed: true, counter, error });
      }
    });
    socket.on('close', () => {
      if (!closed) {
        closed = true;
        clearTimeout(timer);
        resolve({ closed: true, counter });
      }
    });
    socket.on('connect', () => {
      socket.write(Buffer.from([0xef]));
      const pump = () => {
        if (closed) return;
        if (counter >= maxRequests) {
          clearTimeout(timer);
          closed = true;
          socket.destroy();
          resolve({ closed: false, counter });
          return;
        }
        socket.write(reqPqFrame(counter));
        counter += 1;
        setTimeout(pump, 10);
      };
      pump();
    });
  });
}

function idleConnection(idleMs) {
  return new Promise((resolve) => {
    const socket = net.connect(PORT, HOST);
    let closed = false;
    const timer = setTimeout(() => {
      if (!closed) {
        closed = true;
        socket.destroy();
        resolve(false);
      }
    }, idleMs);
    socket.on('connect', () => socket.write(Buffer.from([0xef])));
    socket.on('close', () => {
      if (!closed) {
        closed = true;
        clearTimeout(timer);
        resolve(true);
      }
    });
    socket.on('error', () => {
      if (!closed) {
        closed = true;
        clearTimeout(timer);
        resolve(true);
      }
    });
  });
}

async function healthOk() {
  try {
    const response = await fetch(HEALTH_URL);
    return response.ok;
  } catch {
    return false;
  }
}

async function main() {
  const result = await handshakeUntilClosed();
  if (!result.closed) {
    console.error(`fail rate limit: сервер принял ${result.counter} req_pq и не закрыл соединение`);
    process.exitCode = 1;
  } else {
    console.log(`  ok   handshake rate limit: соединение закрыто после ${result.counter} req_pq`);
  }

  if (!(await healthOk())) {
    console.error('fail: сервер не отвечает после rate limit');
    process.exitCode = 1;
  } else {
    console.log('  ok   /health после rate limit');
  }

  if (EXPECT_IDLE_TIMEOUT) {
    const idleTimeoutMs =
      Number(process.env.KOSTA_IDLE_TIMEOUT_SECS || 2) * 1000 + 3000;
    const closed = await idleConnection(idleTimeoutMs);
    if (!closed) {
      console.error('fail idle timeout: соединение не закрылось');
      process.exitCode = 1;
    } else {
      console.log('  ok   idle timeout: соединение закрыто');
    }
  } else {
    console.log('  skip idle timeout (KOSTA_EXPECT_IDLE_TIMEOUT=1 и короткий idle_timeout_secs)');
  }

  if (process.exitCode) {
    console.error('LIMITS FAILED');
  } else {
    console.log('LIMITS OK');
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});