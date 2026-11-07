# KoSta

KoSta is an experimental messenger built on its own TL-like binary protocol with an
MTProto-inspired handshake. The workspace contains the protocol core, crypto primitives,
transports, a client library, a Rust server (SQLite + web API + WebSocket) and a browser
client.

> **Status:** research/hobby project. Not production-ready — see
> [Known limitations](#known-limitations).

## Requirements

- Rust with edition 2021 support (tested with 1.99)
- Node.js 22+ for the smoke scripts (native `fetch` and `WebSocket`)
- Linux only: `libssl-dev` and `pkg-config` for `native-tls`

## Build and test

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Run

The server reads `config.toml` from the current directory: the binary protocol listens on
`127.0.0.1:8080`, the web client is served on `http://127.0.0.1:8081`.

```bash
cargo run -p kosta-server
```

Open <http://127.0.0.1:8081> and sign in with a phone number and a password; an unknown
number registers a new user.

## End-to-end checks

Protocol round trip (handshake → sign-up → dialog → message → history):

```bash
KOSTA_PHONE=+79990000000 cargo run -p kosta --example test_client
```

Web API and WebSocket (token auth, authorization checks, message delivery, logout):

```bash
node scripts/smoke.mjs --phone=+79990000000
```

TL parser robustness (malformed frames must not crash the server):

```bash
node scripts/tl-garbage.mjs
```

Server limits (handshake rate limit; idle timeout when enabled):

```bash
node scripts/limits.mjs
# with the server running with a short idle_timeout_secs:
KOSTA_EXPECT_IDLE_TIMEOUT=1 node scripts/limits.mjs
```

## Authentication

- **Web**: `POST /api/login` returns a session token (32 random bytes, hex). The database
  stores only its SHA-256; the token lives for 7 days and can be revoked with
  `POST /api/logout`. Send it as `Authorization: Bearer <token>`.
- **WebSocket**: the first message must be
  `{"action":"auth","payload":{"token":"..."}}`.
- **Binary protocol**: MTProto-inspired DH handshake; afterwards messages are encrypted
  with AES-256-CTR and authenticated with HMAC-SHA256, using per-direction keys derived
  with HKDF.

## Server limits

Configured in `config.toml`:

| Option | Default | Meaning |
| --- | --- | --- |
| `idle_timeout_secs` | 300 | connection is closed after this much silence |
| `max_connections` | 256 | excess connections are dropped immediately |

Additionally, the server caps the handshake store at 10 000 pending handshakes with a 60 s
TTL, and rate-limits handshakes to 30 per minute per IP.

## TLS

Certificate and hostname verification are enabled by default. For a local server with a
self-signed certificate there is an explicit development flag:

```bash
KOSTA_USE_TLS=1 KOSTA_TLS_INSECURE=1 cargo run -p kosta --example test_client
```

Enable TLS on the server with `use_tls = true` and `cert_path` / `key_path` pointing at a
PEM certificate and a PKCS#8 private key. `KOSTA_HOST` and `KOSTA_PORT` override the
client target address.

## Session secrets at rest

Session keys in SQLite are encrypted with AES-256-GCM. The master key is generated on the
first start in `session_master.key` (32 bytes, mode `0600`); override the path with
`KOSTA_MASTER_KEY_PATH`.

- Keep the master key separate from the database (secret manager, not next to DB backups).
- Losing the master key makes existing sessions unreadable; users simply have to sign in
  again.
- Databases created before encryption are still readable and get re-encrypted on the next
  session save.

## Configuration

```toml
[server]
host = "127.0.0.1"
port = 8080
db_path = "kosta.db"
use_tls = false
cert_path = "cert.pem"
key_path = "key.pem"
idle_timeout_secs = 300
max_connections = 256
```

## Layout

| Crate | Purpose |
| --- | --- |
| `kosta-core` | TL types and serialization |
| `kosta-crypto` | DH, AES-CTR/HMAC, AES-GCM, HKDF, padding |
| `kosta-transport` | abridged / TCP / TLS / mock transports |
| `kosta` | client session: handshake and encrypted messaging |
| `kosta-server` | server: binary protocol, SQLite, web API and WebSocket |

`web-ui/` holds the single-file browser client; `scripts/` holds the Node.js smoke tests.

## Known limitations

- No TLS handshake tests in CI (they need certificates).
- No Android/desktop clients; only the example client and the web UI.
- No protocol versioning or backwards-compatibility guarantees.
