# KoSta

KoSta is an experimental messenger built on its own TL-like binary protocol with an
MTProto-inspired handshake. The workspace contains the protocol core, crypto primitives,
transports, a client library, a Rust server (SQLite + web API + WebSocket) and a browser
client.

> **Status:** research/hobby project. Not production-ready — see
> [Known limitations](#known-limitations).

## Requirements

- Rust with edition 2021 support (tested with 1.99)
- Node.js 24+ for building the web UI and for the smoke scripts (native `fetch` and
  `WebSocket`; Vite loads its TypeScript config via Node's native type stripping)
- Linux only: `libssl-dev` and `pkg-config` for `native-tls`

## Build and test

```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

## Run

The server reads `config.toml` from the current directory: the binary protocol listens on
`127.0.0.1:8080`, the web client is served on `http://127.0.0.1:8081` (both configurable).
The web client is a Vite/React app, so it has to be built before the server can serve it:

```bash
cd web-ui
npm ci
npm run build
cd ..
cargo run -p kosta-server
```

Open <http://127.0.0.1:8081> and sign in with a phone number and a password; the
"register" mode creates a new user, the "sign in" mode only accepts existing accounts.
`web.ui_dir` in `config.toml` points at the built `web-ui/dist` directory
(override with `KOSTA_WEB_UI_DIR` if the build lives elsewhere).

## Web UI development

Run the API server and the Vite dev server in two terminals:

```bash
cargo run -p kosta-server

cd web-ui
npm install
npm run dev   # http://127.0.0.1:5173, proxies /api and /ws to :8081
```

The UI lives in `web-ui/src` (React + TypeScript + Tailwind v4). `npm run build`
produces `web-ui/dist`, which is what the Rust server serves. The API/WebSocket contract
is described in [docs/web-api.md](docs/web-api.md).

UI features: login/registration, chat list with search and unread badges, private chats
and groups, group administration (admin-only add/remove, rename, leave), online status,
typing indicators, read receipts, message history pagination, light/dark theme and a
responsive mobile layout.

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

- **Web**: `POST /api/login` with `{"phone", "password"}` for an existing account, or
  `{"phone", "password", "register": true}` to also create a new account. Unknown phone
  without `register` returns `401` (this prevents account enumeration). Phone format:
  optional `+` and 5–20 digits; password: 8–128 characters.
  The response contains a session token (32 random bytes, hex); the database stores only
  its SHA-256. The token lives for 7 days and can be revoked with `POST /api/logout`
  (which also closes that user's WebSocket connections).
- **WebSocket**: the first message must be
  `{"action":"auth","payload":{"token":"..."}}`. Tokens are re-validated every 30 seconds;
  revoked tokens close the connection. A small per-IP rate limit applies to auth attempts
  and message sending.
- **Binary protocol**: MTProto-inspired DH handshake; afterwards messages are encrypted
  with AES-256-CTR and authenticated with HMAC-SHA256 (encrypt-then-MAC), using per-direction
  keys derived with HKDF. Client and server share a pinned Ed25519 server key; if
  `server_public_key.der` is missing, the client fails with an explicit error instead of
  silently generating a wrong key.
- **Authorization**: every message/history/dialog operation checks that the caller is a
  participant of the dialog. Group operations (add/remove) require group admin rights;
  leaving a group is allowed to any member. The creator of a group becomes its admin.

## Server limits

Configured in `config.toml`:

| Option | Default | Meaning |
| --- | --- | --- |
| `idle_timeout_secs` | 300 | connection is closed after this much silence |
| `max_connections` | 256 | excess connections are dropped immediately |
| `web.host` / `web.port` | 127.0.0.1 / 8081 | web API, WebSocket and static UI |
| `web.ui_dir` | `web-ui` | directory with `index.html` (`KOSTA_WEB_UI_DIR` overrides) |

Additionally, the server caps the handshake store at 10 000 pending handshakes with a 60 s
TTL, rate-limits handshakes to 30 per minute per IP, logins to 10 per minute per IP,
registrations to 5 per hour per IP and WebSocket messages to 10 per second per user.
Abandoned sessions (completed handshakes that never signed up) are purged after an hour,
and sessions idle for more than 30 days are removed by an hourly cleanup task.

## TLS

Certificate and hostname verification are enabled by default. For a local server with a
self-signed certificate there is an explicit development flag:

```bash
KOSTA_USE_TLS=1 KOSTA_TLS_INSECURE=1 cargo run -p kosta --example test_client
```

Enable TLS on the server with `use_tls = true` and `cert_path` / `key_path` pointing at a
PEM certificate and a PKCS#8 private key. `KOSTA_HOST` and `KOSTA_PORT` override the
client target address.

## Server key and session secrets at rest

- The server's Ed25519 signing key is generated on first start into `server_key.der`
  (mode `0600` on unix; on Windows file ACLs must be restricted separately) and the
  matching public key is written to `server_public_key.der`. On every start the public
  file is re-derived from the private one, so a stale/missing public key heals itself.
  Copy `server_public_key.der` to client hosts. Override paths with
  `KOSTA_SERVER_KEY_PATH` / `KOSTA_SERVER_PUBKEY_PATH` (client reads the public path).
- Session keys in SQLite are encrypted with AES-256-GCM. The master key is generated on the
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

[web]
host = "127.0.0.1"
port = 8081
ui_dir = "web-ui"
```

The database schema is migrated on startup (new columns for media, group admins and
session timestamps), foreign keys are enabled, and SQLite runs in WAL mode.

## Layout

| Crate | Purpose |
| --- | --- |
| `kosta-core` | TL types and serialization |
| `kosta-crypto` | DH, AES-CTR/HMAC, AES-GCM, HKDF, padding, DH parameter checks |
| `kosta-transport` | abridged / TCP / TLS / mock transports |
| `kosta` | client session: handshake and encrypted messaging |
| `kosta-server` | server: binary protocol, SQLite, web API and WebSocket |

`web-ui/` holds the React (Vite + TypeScript) browser client; `docs/` holds the web
API/WebSocket contract; `scripts/` holds the Node.js smoke tests.

## Known limitations

- The protocol's handshake and its KDF are MTProto-*inspired* but not a reviewed
  implementation of MTProto; treat the crypto as unaudited.
- No TLS handshake tests in CI (they need certificates).
- No Android/desktop clients; only the example client and the web UI.
- No protocol versioning or backwards-compatibility guarantees.
- The binary protocol has no error object: a handler error is logged and the connection
  stays open, so a client waiting for a reply can only rely on its own timeout.
- The web UI requires a build step (`npm run build`); the Rust server serves `web-ui/dist`
  and needs Node.js 24+ for the Vite build.
- An authenticated user can check whether an exact phone number is registered
  (`/api/users/lookup`, rate-limited); there is no contact discovery beyond that.
- Groups have a minimal role model (admins can add/remove members); there is no
  ban list, message editing/deletion in the protocol yet, and media upload is not exposed
  by the web API.
