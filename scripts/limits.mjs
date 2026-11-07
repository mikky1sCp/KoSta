#!/usr/bin/env node
// Server limits regression test.
// Requires a running server on 127.0.0.1:8080.
//
//   node scripts/limits.mjs
//
// Checks:
//   1. Handshake rate limit: the 31st ReqPq from one IP must drop the connection.
//   2. Idle timeout (optional): a silent connection must be closed by the server.
//      Only meaningful when the server runs with a short idle_timeout_secs, so the
//      check is enabled explicitly:
//
//        KOSTA_EXPECT_IDLE_TIMEOUT=1 node scripts/limits.mjs
//
// The script does not touch TLS: it speaks the abridged frame format directly.

import net from 'node:net';

const HOST = process.env.KOSTA_HOST ?? '127.0.0.1';
const PORT = Number(process.env.KOSTA_PORT ?? 8080);
const EXPECT_IDLE_TIMEOUT = process.env.KOSTA_EXPECT_IDLE_TIMEOUT === '1';
const REQ_PQ_ID = 0x60469778;

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const once = (emitter, event) => new Promise((resolve) => emitter.once(event, resolve));

function frame(payload) {
  if (payload.length <= 0x7e) {
    return Buffer.concat([Buffer.from([payload.length]), payload]);
  }
  const header = Buffer.alloc(4);
  header[0] = 0x7f;
  header.writeUIntLE(payload.length, 1, 3);
  return Buffer.concat([header, payload]);
}

function u32le(value) {
  const buffer = Buffer.alloc(4);
  buffer.writeUInt32LE(value >>> 0, 0);
  return buffer;
}

let failures = 0;

function check(name, ok, detail = '') {
  if (ok) {
    console.log(`  ok   ${name}`);
  } else {
    failures += 1;
    console.log(`  FAIL ${name}${detail ? ` — ${detail}` : ''}`);
  }
}

// 1. Handshake rate limit.
{
  const socket = net.connect(PORT, HOST);
  await once(socket, 'connect');
  // Without resume() incoming data is never read and the server's FIN is not processed.
  socket.resume();

  let dropped = false;
  socket.on('close', () => {
    dropped = true;
  });
  socket.on('error', () => {
    dropped = true;
  });

  const reqPq = frame(Buffer.concat([u32le(REQ_PQ_ID), Buffer.alloc(16)]));
  for (let i = 0; i < 31; i += 1) {
    socket.write(reqPq);
    await sleep(10);
  }
  await sleep(1500);

  check('handshake rate limit drops the connection', dropped);
  socket.destroy();
}

// 2. Idle timeout (opt-in, needs a short idle_timeout_secs on the server).
if (EXPECT_IDLE_TIMEOUT) {
  const socket = net.connect(PORT, HOST);
  await once(socket, 'connect');
  socket.resume();

  const closed = await Promise.race([
    once(socket, 'close').then(() => true),
    sleep(10_000).then(() => false),
  ]);
  check('idle connection is closed by the server', closed);
  socket.destroy();
} else {
  console.log('  skip idle timeout (set KOSTA_EXPECT_IDLE_TIMEOUT=1 and a short idle_timeout_secs)');
}

if (failures > 0) {
  console.log(`\n${failures} failed`);
  process.exit(1);
}
console.log('\nLIMITS OK');
