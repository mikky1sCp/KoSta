# KoSta migration to MTProto 2.0: design and phased plan

Status: **completed** (stages 0–6). The binary protocol is MTProto 2.0 and the
legacy layer has been removed. This document is kept as a design record and a
migration log.

The original goal was to gradually replace the hand-written message layer with
MTProto 2.0 while keeping the project working at every step.

## 0. Decisions

| Question | Decision |
| --- | --- |
| Target protocol | MTProto 2.0: AES-256-IGE + `msg_key`, containers, `msg_id`/`seq_no`, acks, `rpc_result`/`rpc_error` |
| TL layer | Our **own** TL schema (not Telegram's): Rust code generated from `schema/kosta.tl` |
| Web UI and REST/WS | Unchanged. Migration covers only the binary protocol (port 8080) |
| Handshake | Our own (Ed25519 pinning + DH-2048), but TL objects use canonical ids |
| Migration | Incremental, dual-stack: legacy protocol on 8080, MTProto 2.0 on port 8082 until stage 6 |
| Telegram compatibility | Not a goal. “Spec-shaped”, not “compatible with Telegram clients” |

## 1. State before the migration (as-is)

### 1.1 Layers

| Layer | Where | Then |
| --- | --- | --- |
| Transport | `kosta-transport/src/abridged.rs`, `tcp.rs` | “Abridged”, but **not MTProto-abridged**: no `0xef` init byte, length encoded in bytes rather than 4-byte words, payload not required to be a multiple of 4 |
| Message encryption | `kosta-crypto/src/session_crypto.rs`, `kosta/src/session.rs:448-544`, `kosta-server/src/server.rs:240-300` | AES-256-CTR + HMAC-SHA256 (truncated to 16 bytes), encrypt-then-MAC; nonce = `u32` counter + 12 random bytes; frame: `auth_key_id:8 \| nonce:16 \| ciphertext \| tag:16` |
| KDF | `session_crypto::derive_keys` | HKDF-SHA256(shared_secret, salt = SHA256(client_nonce ‖ server_nonce), info = `"kosta-keys"`) → 4 keys |
| Handshake | `kosta/src/session.rs:245-445`, `kosta-server/src/handlers.rs` | req_pq → resPQ → req_DH_params → server_DH_params_ok → set_client_DH_params → dh_gen_ok; DH-2048; Ed25519 server signature + pinned public key; inner data — AES-GCM |
| TL | `kosta-core/src/tl/types.rs` (472 lines), `constructors.rs` (842 lines) | Hand-written types and serialization; handshake ids are canonical (`req_pq#60469778`, `req_DH_params#d712e4be`, …), application ids were invented (`0x1a2b3c4d` etc.) |
| Messages | `kosta/src/session.rs` | The encrypted body carries `msg_id:i64`, `seq_no:i32`, then a TL object. `msg_id` was an arbitrary counter (not a multiple of 4, low 32 bits not time-based), ±300 s window; `seq_no` was a simple increment |
| RPC | `kosta-server/src/handlers.rs` | No correlation object: the response was inferred from order/TL type. An error meant “log and stay silent” (see README, “Known limitations”) |
| Acks / containers | — | Absent |
| Sessions | `kosta-server/src/session_cache.rs`, `kosta-server/src/db.rs` | SQLite, session keys encrypted with AES-256-GCM (`session_master.key`); counters and salt stored in the DB |

### 1.2 What had to be broken deliberately

The old transport was not compatible with MTProto-abridged, so the transport
and the message layer had to change together on both client and server. Hence
the decision to use a **dual-stack setup on a separate port** (see § 4) instead
of trying to “blend” the formats.

### 1.3 Audit: what the migration had to account for

- **`auth_key` was wrong.** `AuthKey::from_shared_secret` built a SHA-256 chain
  of 8 blocks (`kosta-crypto/src/keys.rs:44-68`), and `auth_key_id` was computed
  over `shared_secret.to_bytes_le()` of variable length
  (`kosta/src/session.rs:40-48`). MTProto 2.0 requires: `auth_key` = raw
  2048-bit DH secret (256 bytes big-endian), `auth_key_id = SHA1(auth_key)[12..20]`
  as LE i64. Therefore all existing SQLite sessions were invalid for MTProto →
  stage 4 needed a session reset / forced re-handshake (`protocol_version`
  column).
- **The handshake had no unencrypted envelope.** The frame payload was a bare
  TL boxed object; in MTProto even a plaintext message has
  `auth_key_id = 0 | msg_id | msg_len | body`.
- **The PQ step was decorative.** In MTProto `resPQ` does not contain `p`/`q`:
  the client factors `pq` itself and sends the resulting `p`/`q` in
  `req_DH_params`. Here the server sent `p` and `q` right in `resPQ`, the client
  echoed them, and `pq` was only 32-bit — i.e. no proof of work/factorization;
  `retry_id` was ignored, `server_DH_params_fail`/`dh_gen_fail` were missing,
  the `resPQ` id was invented (`0x9a2b3c4d` instead of `0x05162463`), and
  `new_nonce_hash1` was derived from the shared secret rather than from
  `SHA1(new_nonce + 1 + server_nonce)`.
- **TL encoding was not canonical.** Strings/bytes were canonical (1/4-byte
  header + padding), but `vector` was written without the `0x1cb5c415` id,
  `bool` as i32 0/1 (instead of `0x997275b5`/`0xbc799737`), and `Option<T>` as
  an i32 flag (in TL this is expressed with flags bits in the constructor
  declaration). Our own schema could keep that, but the generator had to support
  the canonical encoding.
- **Salt/session_id were absent on the wire.** `server_salt` was taken from
  `server_nonce`, the server always wrote 0 and never checked it; `session_id`
  was not transmitted at all (`session.rs:74`, `:117`).
- **Errors were silent.** `Option<Vec<u8>>` + `continue` (`server.rs:303-312`):
  no response on error, so the client waited for its read timeout.
- **Persistence on every message** (`session_cache.rs:88-140`) would become a
  bottleneck with containers and batching.
- **Replay LRU was not persisted** (`session_cache.rs:55-57`): the window was
  lost after a restart. MTProto requires returning the cached response for a
  duplicate instead of dropping it (the old code dropped it,
  `server.rs:265-280`).
- **`kosta-crypto/src/keys.rs:8-42`** was a `msg_key`/`kdf` stub with wrong
  formulas; it could not be reused and was removed in stage 6.

## 2. Target protocol (MTProto 2.0 subset)

### 2.1 Transport: spec TCP-abridged

- The first byte of a connection is `0xef`.
- Each frame: `len/4` (payload length in 4-byte words):
  - `len/4 < 0x7f` → one byte;
  - otherwise `0x7f` + 3 little-endian bytes.
- Requirements: `len % 4 == 0`; `len <= 1 MiB` (kept the previous limit).
- The TLS wrapper (`kosta-transport/src/tls.rs`) was kept.

### 2.2 Encrypted envelope

```
auth_key_id : long            (8 bytes)
msg_key     : int128          (16 bytes)
encrypted_data : AES-256-IGE(padded_plaintext)   (16 * k bytes)
```

`padded_plaintext` = `salt:long | session_id:long | msg_id:long | seq_no:int |
body:TL` + random padding to a multiple of 16 bytes; padding length is
12..1024 bytes. `salt` and `session_id` are part of the plaintext (there are no
separate header fields for them).

`auth_key` is the raw 2048-bit DH secret, exactly 256 bytes **big-endian**
(previously it was a hash / variable-length little-endian value).
`auth_key_id` = the first 8 bytes of `SHA1(auth_key)[12..20]` (LE i64).

KDF (the `x` constants are isolated in code as `X_CLIENT = 0`, `X_SERVER = 8`):

```
msg_key_large = SHA256(auth_key[88+x .. 88+x+32] ‖ padded_plaintext)
msg_key       = msg_key_large[8 .. 24]

sha256_a = SHA256(msg_key ‖ auth_key[x .. x+36])
sha256_b = SHA256(auth_key[40+x .. 40+x+36] ‖ msg_key)

aes_key = sha256_a[0..8] ‖ sha256_b[8..24] ‖ sha256_a[24..32]
aes_iv  = sha256_b[0..8] ‖ sha256_a[8..24] ‖ sha256_b[24..32]
```

`x = 0` for client → server, `x = 8` for server → client.

Integrity: there is no MAC tag. After decryption the server/client
**recomputes** `msg_key` over the decrypted plaintext and compares it with the
received one (constant-time). A mismatch means the frame is dropped. This is
MTProto 2.0 message authentication.

> ⚠️ The formulas above were reconstructed from memory (the sandbox had no
> network access). Before stage 4 they should be verified against
> `core.telegram.org/mtproto/description`; in code they live in a single module
> `kosta-crypto/src/mtproto2.rs` with constants and KATs, so a fix is localized.

### 2.2.1 Unencrypted messages (handshake)

While there is no `auth_key` yet, a frame carries a plaintext envelope:

```
auth_key_id : long = 0
msg_id      : long
msg_len     : int
body        : TL boxed
```

The `msg_id` here is time-based too (low bits are multiples of 4); no acks or
salt are required. This differs from the old code, where the handshake was a
bare TL object without an envelope.

### 2.3 Inner layer

- `msg_id:long` = `unix_seconds << 32 | (n << 2)` — the low 32 bits are
  multiples of 4. Client `msg_id % 4 == 0`, server `msg_id % 4 == 1`; it grows
  monotonically within a session. A container carries its own `msg_id`, and the
  inner messages carry theirs.
- `seq_no:int`: content-related messages — `2*n + 1`, others — `2*n`, container
  — `0`. Content-related messages require an ack; non-content do not.
- Duplicates: when `msg_id <= last_recv_msg_id` but the id is still in the
  duplicate window, the sender receives **the same** `rpc_result` again instead
  of a silent drop (the old code just dropped the frame, `server.rs:265-280`).
- Container: `msg_container#73f1f8dc messages:vector<message>`,
  `message#5bb8e511 msg_id:long seqno:int bytes:int body:bytes`. Limits: up to
  1024 messages, nested containers forbidden. `msg_id` validation: inner
  messages first, then the envelope's own `msg_id` (it is generated later).
- `session_id` is a random 64-bit identifier; the client generates it when
  creating the session and sends it in the plaintext of every message, so the
  server can find the state. `salt` — see § 2.4.
- `msgs_ack#62d6b459 msg_ids:vector<long>` — delivery acknowledgement.
- `rpc_result#f35c6d01 req_msg_id:long result:Object` — request/response
  correlation; `rpc_error#2144ca19 error_code:int error_message:string` —
  errors (this closes the known “no error object” limitation).
- The server always wraps responses in a `msg_container`:
  `[new_session_created?] + [msgs_ack?] + rpc_result(s)`. `new_session_created`
  is sent once per connection (in the first response); afterwards the salt is
  accepted as current/previous.
- A duplicate `msg_id` (while the response is still cached) receives the cached
  `rpc_result` and does not re-run the operation.
- `gzip_packed#3072cfa1` — not in the MVP (stage 7).
- Also: `ping#7abe77ec` / `pong#347773c5`, `bad_msg_notification#a7eff811`,
  `new_session_created#9ec20908`.

### 2.4 Salt

`salt` does not come from the handshake: it arrives in
`new_session_created#9ec20908` (`first_msg_id`, `unique_id`, `server_salt`).
The client starts with `salt = 0`; the server accepts the current and previous
salt (transition window around a salt change). `get_future_salts` /
`future_salts` are not implemented — a deliberate deviation. When the salt
changes, the server sends a new `new_session_created`.

### 2.5 `bad_msg_notification` codes

Planned: 16/17 (`msg_id` too low/high), 18 (not a multiple of 4), 19/20
(invalid container/seq_no) with client-side `msg_id` correction. In the current
MVP (stage 4) an invalid `msg_id`/salt closes the connection — a deliberate
temporary deviation until stage 5/7.

### 2.6 Explicit deviations from the spec (documented in README)

1. Handshake: Ed25519-pinned key instead of Telegram's RSA signature; the
   handshake inner data stays AES-GCM (the spec uses AES-IGE/RSA), and the
   Ed25519 signature remains a separate `signature` field in
   `server_DH_params_ok`. We do not plan to change this: Telegram compatibility
   is not a goal, and the current scheme is no worse. The handshake shape is
   nevertheless aligned with the spec (stage 4): `resPQ` without `p`/`q`,
   `pq` is 64-bit (client factors it), `p`/`q` in `req_DH_params`,
   `server_DH_params_fail` / `dh_gen_fail`, `retry_id`.
2. Our own TL schema and namespace (`kosta.*`), not `api.tl`.
3. No future salts and no obfuscated2 (TLS is available); a single `server_salt`.
4. `gzip_packed` — later.
5. PQ proof-of-work is done for real in stage 4: `pq` is 64-bit, the client
   factors it (Pollard rho), and `p`/`q` are sent in `req_DH_params`. Additional
   DoS protection at this step is a candidate for stage 7.
6. An invalid `msg_id`/salt in the MVP closes the connection (no
   `bad_msg_notification`); time auto-correction is stage 5.

## 3. Architecture after the migration

Stages 2–6 are implemented: `kosta-protocol`, `kosta-tl` + `kosta-tl-gen`,
the MTProto 2.0 server listener on `server.port`; `kosta-core` is removed.

```
schema/kosta.tl ──► kosta-tl-gen ──► kosta-tl (generated)
                                         ▲
services                                 │
  kosta-server ──► kosta-protocol ──► kosta-crypto (IGE + KDF)
       │                └► kosta-transport (abridged/TCP/TLS)
       └► web/* (REST + WS) — UNCHANGED

  kosta (client) ──► kosta-protocol ──► …
```

| Crate | What appears | What goes away |
| --- | --- | --- |
| `kosta-crypto` | `ige.rs` (AES-256-IGE), `mtproto2.rs` (KDF, sealing/open, padding) | `session_crypto.rs`, `ctr.rs`, `keys.rs` — removed in stage 6 |
| `kosta-transport` | real MTProto-abridged + tests | — |
| `kosta-protocol` (**new**, ✅ stage 2) | `msg_id`, `seq_no`, frames, containers, `msgs_ack`, `rpc_result`/`rpc_error`, salt, dedup/response cache | — |
| `kosta-tl-gen` (**new**, ✅ stage 3) | `.tl` parser, CRC32 ids, Rust code generator | hand-written TL — removed in stage 6 |
| `kosta-tl` (**new**, ✅ stage 3) | TL primitives (`Bytes`, `Reader`, `Bool`, `vector`), generated types and `TlObject` | `kosta-core/src/tl/*` — removed in stage 6 |
| `kosta-core` | removed in stage 6; golden vectors kept in `kosta-tl` | — |
| `kosta` | ✅ stage 5: `MtprotoSession` on `kosta-protocol` (RPC, acks, retry, TLS) | old `Session` — removed in stage 6 |
| `kosta-server` | ✅ stage 4/6: MTProto 2.0 on `server.port`, dispatch via `kosta-tl`, `mtproto_sessions` table | legacy modules — removed in stage 6 |

The TL schema (`schema/kosta.tl`) has 45 constructors: handshake (canonical
ids), envelope (`msg_container`, `message`, `msgs_ack`, `rpc_result`,
`rpc_error`, `ping`/`pong`, `bad_msg_notification`, `new_session_created`) and
application RPCs (`sendMessage`, `getHistory`, `getDialogs`,
`createPrivateDialog`, …). Canonical objects have explicit ids; our own are
computed by the generator as CRC32 over the normalized declaration. Golden tests
check both the known ids and byte-for-byte handshake compatibility with the
(pre-removal) `kosta-core`.
Encoding is canonical TL: `vector#1cb5c415`, `true`/`false`
(`0x997275b5`/`0xbc799737`), `bytes`/`string` with a 1/4-byte header + padding.
Flags bits are not used: optionality is modelled with constructors
(`chatMessageEmpty`).

## 4. Migration strategy (dual-stack, historically)

1. Before the cutover the legacy protocol stayed on 8080 and MTProto 2.0 ran as
   a separate listener on 8082.
2. During migration `config.toml` had:
   ```toml
   [mtproto]
   enabled = true
   host = "127.0.0.1"
   port = 8082
   ```
3. `kosta-server` started a second listener; business logic (`db.rs`,
   `web/*`, `media.rs`) was reused.
4. Client: `kosta::MtprotoSession` — the new client layer (stage 5); e2e in
   `kosta/examples/mtproto_client.rs` (`KOSTA_PORT`, now 8080).
5. Cutover (stage 6) is done: MTProto 2.0 now runs on `server.port` (8080), the
   `[mtproto]` block and port 8082 are gone, legacy code and `kosta-core` are
   removed.

Dual-stack proved convenient: transport, handshake and containers were verified
on a separate port without breaking a working client; once stable, the switch
was just moving the listener to `server.port` and deleting the old modules.

## 5. Stages and definition of done

Audit estimate: ~2000–2500 changed lines + ~1400–1800 new ones, 6–10
person-days excluding crypto review; with tests and debugging — closer to
2–3 weeks of unhurried work.

### Stage 0. Design and plan (this document)

- [x] Decisions recorded in § 0.
- [x] Stage plan and DoD.
- DoD: document in the repository, decisions approved by the project owner.

### Stage 1. MTProto 2.0 crypto primitives (0.5–1 day) — ✅ done

Files: `kosta-crypto/src/ige.rs`, `kosta-crypto/src/mtproto2.rs`,
`kosta-crypto/src/lib.rs`, `scripts/mtproto2-vectors/gen_vectors.py`.

- [x] AES-256-IGE (encrypt/decrypt) based on `aes`/`cipher`.
- [x] KDF (`msg_key`, `aes_key`/`aes_iv`), `auth_key_id`, message sealing/opening
      with 12..1024 padding and leftover validation.
- [x] `msg_key` verification on open (constant-time, `subtle`).
- [x] KATs from an independent Python implementation (pycryptodome): IGE and KDF
      for both directions; generator in `scripts/mtproto2-vectors/`.
- [x] Negative tests: broken `msg_key`, broken ciphertext, wrong direction,
      wrong `auth_key`, invalid lengths, padding boundaries.
- DoD met: `cargo test -p kosta-crypto` — 26 passed; `cargo test --workspace` —
  100 passed; `clippy -D warnings` clean; existing call sites unchanged.

### Stage 2. `kosta-protocol` crate (2–3 days) — ✅ done

Files: `kosta-protocol/Cargo.toml`, `src/{lib,frame,tl,container,ack,rpc,msg_id,salt,error}.rs`,
`tests/flow.rs`.

- [x] Frames: the plaintext handshake envelope and the encrypted frame
      (`auth_key_id | msg_key | AES-256-IGE`), plaintext
      `salt | session_id | msg_id | seq_no | body | padding`.
- [x] Canonical TL encoding: `bytes`/`string` with padding, `vector#1cb5c415`,
      bounds and protection against huge declared lengths.
- [x] Containers: build/parse, limits (1..=1024 messages, 1 MiB), empty and
      nested containers rejected, `msg_id` validation order.
- [x] `msgs_ack` + `AckTracker`, `ResponseCache` for repeated requests.
- [x] `rpc_result`/`rpc_error` with `req_msg_id` correlation (closes the
      “no error object” item from Known limitations).
- [x] `new_session_created`, `ServerSalt` with current/previous salt window,
      salt/session_id generation.
- [x] `msg_id`/`seq_no` with MTProto semantics: client `% 4 == 0`, server
      `% 4 == 1`, content-related `seq_no = 2n+1`, time window −300 s / +30 s.
- [x] Integration scenario “request → response container (ack + salt + rpc) →
      repeated request from cache” on a fixed key.
- The retry queue for unacknowledged content messages moved to stage 5
  (it needs the client event loop).
- DoD met: `cargo test -p kosta-protocol` — 45 passed (42 unit + 3 integration),
  `clippy -D warnings` clean, `cargo fmt` applied.

### Stage 3. TL schema and code generator (2–4 days) — ✅ done

Files: `schema/kosta.tl` (45 constructors), `kosta-tl-gen/` (parser, CRC32,
generator), `kosta-tl/` (primitives + generation into `OUT_DIR` via `build.rs`),
`kosta-tl/tests/golden.rs`.

- [x] Syntax: constructors, functions, `vector<T>`, `int/long/int128/int256/
      double/string/bytes/Bool/Object`, bare types, explicit `#id` and auto ids.
- [x] Generation: `struct` for single-constructor types, `enum` variants for
      multi-constructor ones, `TlWrite`/`TlRead`, `TlObject`, `constructor_id`,
      `read_boxed`/`write_boxed`.
- [x] Canonical encoding: `vector#1cb5c415`, `true`/`false`; `bytes`/`string`
      with padding. Flags bits were not needed.
- [x] Handshake schema: `resPQ#05162463` without `p/q`, `p/q` in
      `req_DH_params`, `server_DH_params_fail` / `dh_gen_fail`, `retry_id`; the
      Ed25519 signature remains a separate `signature` field (deliberate
      deviation).
- [x] Golden tests: 8 handshake objects serialize **byte-for-byte** like
      `kosta-core` and read back; canonical ids verified.
- [x] Roundtrip of application objects: `ChatMessage`/`ChatMessageEmpty`,
      `DialogInfo`, `DialogList`, `HistoryResult`, `RpcResult`,
      `msg_container`, `TlObject` dispatch, unknown id.
- DoD met: `cargo test -p kosta-tl` — 20 passed (8 unit + 11 golden +
  1 doc-test), `kosta-tl-gen` — 8 passed; `clippy -D warnings` clean.

### Stage 4. MTProto 2.0 server (2–3 days) — ✅ done

Files: `kosta-server/src/mtproto/{mod,connection,handshake,rpc,session}.rs`,
`kosta-server/src/db.rs` (`mtproto_sessions` table), `kosta-server/src/main.rs`,
`config.toml`, `kosta-transport/src/mtproto_abridged.rs`,
`kosta/examples/mtproto_client.rs`.

- [x] Second listener (8082), real abridged (`0xef` + length in 4-byte words),
      unencrypted handshake envelope, envelope, dispatch via `kosta-tl`.
- [x] `auth_key` = raw DH secret 256 B BE, `auth_key_id = SHA1(auth_key)[12..20]`;
      sessions in the new `mtproto_sessions` table (old `sessions` untouched),
      `auth_key` encrypted with the master key.
- [x] Handshake: `resPQ` without `p`/`q`, 64-bit `pq` + real factorization
      (Pollard rho in `kosta-crypto`), `p`/`q` in `req_DH_params`, canonical ids,
      `new_nonce` taken from `p_q_inner_data`, Ed25519 signature verified by the
      client.
- [x] Server responses — `msg_container` with `new_session_created` (once),
      `msgs_ack` and `rpc_result`; duplicates get the cached response; counters
      are persisted every 32 frames and on connection close.
- [x] Kept `idle_timeout_secs`, `max_connections`, handshake rate limit, 1 MiB
      frame limit; TLS uses the same certificates.
- [x] E2E client `mtproto_client`: handshake → `signUp` → `createGroup` →
      `sendMessage` → `getHistory`; the legacy client on 8080 still worked.
- DoD met: e2e against 8082 green, legacy round trip green,
  `cargo test --workspace` — 188 passed, `clippy -D warnings` clean.
  Moving `scripts/limits.mjs` and the CI step to the new protocol — stage 6.

### Stage 5. MTProto 2.0 client (2 days) — ✅ done

Files: `kosta/src/mtproto.rs`, `kosta/src/lib.rs`, `kosta/src/error.rs`,
`kosta-transport/src/tls.rs`, `kosta/examples/mtproto_client.rs`.

- [x] `kosta::MtprotoSession` — the new client layer on `kosta-protocol`:
      handshake (factor `pq`, verify the Ed25519 signature, raw `auth_key`),
      RPC with `req_msg_id`, `new_session_created` → salt, validation of server
      `msg_id`, `rpc_error`/`bad_msg_notification` as errors.
- [x] Acks: incoming content-related messages are acknowledged with `msgs_ack`
      in the next request (container `[request, msgs_ack]`).
- [x] Retry: on timeout the request is retried with the same `msg_id`, and the
      server returns the cached response.
- [x] TLS: `kosta-transport::tls::connect_tls_stream` + the same abridged; the
      old `connect_tls` reused it.
- [x] High-level methods: `sign_up`, `create_group`, `create_private_dialog`,
      `send_message`, `get_history`, `get_dialogs`, `ack_message`,
      `update_status`.
- [x] The `mtproto_client` example was rewritten on top of the library (no
      duplicated handshake) and passes: handshake → signUp → createGroup →
      sendMessage → ack → status → history → dialogs.
- Deviation: the old `Session` was not removed yet (cutover — stage 6), and
  there was no `KOSTA_PROTOCOL` switch in `test_client`; e2e ran via a separate
  example.
- DoD met: e2e green, `cargo test --workspace` — 191 passed, `clippy -D
  warnings` clean.

### Stage 6. Cutover and legacy removal (1–2 days) — ✅ done

- [x] MTProto 2.0 now runs on `server.host` / `server.port` (8080); the separate
      `[mtproto]` listener and port 8082 are gone.
- [x] Removed legacy layers: `kosta-server/src/{server,handlers,session_cache,
      session_store}.rs`, the `sessions` table and its code in `db.rs`,
      `kosta-core` entirely (hand-written TL types), `kosta/src/session.rs`
      (old client), `kosta-crypto/{session_crypto,ctr,keys}.rs`, and the
      `kosta-core` dependency from every crate.
- [x] `kosta-tl` golden tests moved to static byte vectors captured from
      `kosta-core` before its removal (regression protection preserved).
- [x] `scripts/tl-garbage.mjs` and `scripts/limits.mjs` rewritten for MTProto
      2.0; the CI e2e job runs `mtproto_client`, web smoke, garbage and limits.
- [x] README/doc updated: a single binary protocol — MTProto 2.0; web UI and
      REST/WS are still untouched.
- DoD met: `cargo test --workspace` — 164 passed, `clippy -D warnings` clean;
  `mtproto_client` e2e on 8080 green, web smoke 47/47, garbage and limits green;
  `npx tsc --noEmit` (web-ui) unchanged.

### Stage 7. Optional

Intermediate transport (`0xeeeeeeee`), `gzip_packed`, future salts, load tests,
external crypto review.

## 6. Test strategy

- **Unit/KAT**: IGE and KDF known answers; TL generator golden bytes (handshake
  compared against static vectors, ids against canonical values); containers
  boundary and garbage cases.
- **Cross-impl check**: the Python script (`scripts/mtproto2-vectors/`)
  independently implements IGE + KDF and compares encrypted vectors with Rust.
- **E2E**: `kosta/examples/mtproto_client.rs` on top of `kosta::MtprotoSession`
  (handshake → signUp → group → message → ack/status → history → dialogs),
  `scripts/smoke.mjs` (HTTP/WS), `scripts/limits.mjs`,
  `scripts/tl-garbage.mjs` — all on port 8080.
- **Fuzzing**: a deterministic random harness for frame/container parsing
  (without `cargo-fuzz`, to avoid extra dependencies).
- **CI**: e2e job with the MTProto client, web smoke, garbage and limits.

## 7. Risks

| Risk | Mitigation |
| --- | --- |
| A KDF/IGE bug causes silent message corruption (there is no MAC tag) | KATs, cross-impl, strict `msg_key` verification, integration tests |
| `auth_key` changes → old sessions and counters in SQLite are incompatible | `protocol_version` in `sessions`, reset sessions on first connect, do not migrate `seq_no`/`msg_id` |
| Without a response cache, duplicates/retries break idempotency | `rpc_result` cache by `msg_id` + `random_id` in application RPCs |
| Persisting on every frame becomes a bottleneck with containers | dirty flag / batching, measure before cutover |
| Formulas from memory are wrong (no network) | Isolate in `mtproto2.rs`, mark TODO, verify against the spec before stage 4 |
| Incompatible abridged breaks both sides | Separate port 8082, “old protocol still works” test |
| Codegen complexity | Start with a minimal TL subset, golden tests |
| Dual-stack lives too long | Timebox: no more than 3 weeks from stage 1 to stage 6 |
| Own TL schema ≠ Telegram TL | State explicitly in README; Telegram compatibility is not claimed |

## 8. Open questions

1. Is Telegram TL-schema compatibility needed in the future (in which case the
   generator should be designed for `api.tl` from the start)?
2. Is `gzip_packed` and the intermediate transport needed in the MVP or after
   the cutover?
3. Do we keep the Ed25519 handshake (recommended) or move to the RSA signature
   from the spec in stage 7?

## 9. Execution log

- **2026-11-08.** Stage 0: design doc, audit of the current protocol (see § 1.3).
  Stage 1: `kosta-crypto/src/ige.rs` (AES-256-IGE) and
  `kosta-crypto/src/mtproto2.rs` (KDF, `auth_key_id`, seal/open, padding
  validation); KATs from an independent Python implementation; 14 new tests.
  Result: `cargo test --workspace` — 100 passed, `clippy -D warnings` clean.
  Legacy code untouched.
- **2026-11-08 (continued).** Stage 2: `kosta-protocol` crate — frames
  (unencrypted/encrypted), canonical TL, containers, `msgs_ack` +
  `ResponseCache`, `rpc_result`/`rpc_error`, `new_session_created` and
  `ServerSalt`, `msg_id`/`seq_no` with MTProto semantics; integration scenario
  “request → response → duplicate”. Result: `cargo test -p kosta-protocol` — 45
  passed, `clippy -D warnings` clean. Clarified that `salt`/`session_id` are
  part of the plaintext (per spec); the doc was updated.
- **2026-11-08 (stage 3).** TL layer: `schema/kosta.tl` (45 constructors),
  `kosta-tl-gen` (parser, CRC32 ids, generator), `kosta-tl` (primitives
  `Bytes`/`Int128`/`Int256`/`Reader`, canonical `vector`/`Bool`, codegen into
  `OUT_DIR`). Golden tests: 8 handshake objects byte-for-byte equal to
  `kosta-core`; roundtrip of application types, `TlObject` dispatch.
  Result: `cargo test --workspace` — 173 passed, `clippy -D warnings` clean.
  Clarified that MTProto's `req_DH_params` includes `p`/`q` (only `resPQ` does
  not), so the plan item was corrected.
- **2026-11-08 (stage 4).** MTProto 2.0 server: separate listener 8082,
  `kosta-transport/src/mtproto_abridged.rs` (init `0xef`, length in words),
  handshake with 64-bit `pq` and Pollard factorization, raw `auth_key`,
  `mtproto_sessions` table (encrypted `auth_key`, counters, salt), container
  responses with `new_session_created`/`msgs_ack`/`rpc_result`, duplicate
  response cache, persistence every 32 frames. The e2e client
  `kosta/examples/mtproto_client.rs` passed its scenario; the legacy protocol
  on 8080 kept working. Result: `cargo test --workspace` — 188 passed,
  `clippy -D warnings` clean.
- **2026-11-08 (stage 5).** Client layer `kosta::MtprotoSession`: handshake
  with signature verification and `pq` factorization, RPC with `req_msg_id`,
  acks in the container `[request, msgs_ack]`, retry on timeout with the same
  `msg_id`, TLS via `connect_tls_stream`, high-level methods (`sign_up`,
  `create_group`, `send_message`, `get_history`, `get_dialogs`, `ack_message`,
  `update_status`). The `mtproto_client` example was rewritten on top of the
  library and passes e2e; the legacy client on 8080 kept working.
  Result: `cargo test --workspace` — 191 passed, `clippy -D warnings` clean.
- **2026-11-08 (stage 6, cutover).** MTProto 2.0 moved to `server.port` (8080);
  the separate 8082 listener and the `[mtproto]` block were removed. Removed the
  legacy server modules (`server.rs`, `handlers.rs`, `session_cache.rs`,
  `session_store.rs`), the `sessions` table and its code, the old client
  `kosta/src/session.rs`, `kosta-core` entirely,
  `kosta-crypto/session_crypto`, `ctr`, `keys`; `kosta-transport` no longer
  depends on `kosta-core`. `kosta-tl` golden tests moved to static byte vectors.
  `scripts/tl-garbage.mjs` and `scripts/limits.mjs` were rewritten for MTProto
  2.0, and the CI e2e job was updated. Result: `cargo test --workspace` — 164
  passed, `clippy -D warnings` clean; `mtproto_client` on 8080, web smoke 47/47,
  garbage and limits green. Web UI/REST/WS unchanged.