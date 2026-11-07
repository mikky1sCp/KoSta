# KoSta web API and WebSocket protocol

This document defines the contract between `web-ui` (Vite + React) and the server
(`kosta-server/src/web/`). Every REST endpoint except `POST /api/login` requires an
`Authorization: Bearer <token>` header and returns `401` without one.

The common error shape is `{"error": "<short message>"}`.
Internal server errors always return `500 {"error":"internal server error"}`; details are
written to the log only.

All field names use `snake_case` (consistent with the existing API).

## Authentication

### POST /api/login

```json
{"phone": "+79990000000", "password": "testpass123", "register": true}
```

- `register` defaults to `false`: without it an unknown phone number gets `401`
  (this prevents account enumeration).
- Validation: phone is an optional `+` followed by 5–20 digits; password is 8–128
  characters; otherwise `400`.
- `200` response:

```json
{"success": true, "token": "<64 hex>", "auth_key_id": 42, "user_id": 7, "error": null}
```

### POST /api/logout

Revokes the token and closes the user's WebSocket connections. Returns
`200 {"success": true}`.

## Profile

### GET /api/me

```json
{"user_id": 7, "phone": "+79990000000", "is_online": true}
```

## Chats

### GET /api/chats

An array of `ChatItem`, sorted by the last message time:

```json
{
  "id": 12,
  "title": "+79990000001",
  "is_group": false,
  "last_message": "hello",
  "last_timestamp": 1794057537,
  "unread_count": 2,
  "other_user_id": 8,
  "is_online": true
}
```

- `title` is resolved by the server: the peer's phone number for a private chat, the
  stored title for a group.
- `other_user_id` and `is_online` are present for private chats only.
- `unread_count` is the number of unread incoming messages.

### GET /api/history?chat_id=12&limit=50&offset=0

An array of messages, newest first (`limit` is clamped to 1..=200, `offset >= 0`):

```json
{
  "id": 105,
  "sender_id": 8,
  "text": "hello",
  "timestamp": 1794057537,
  "is_outgoing": false,
  "read": true,
  "delivered": false,
  "media_name": null,
  "media_type": null,
  "media_size": null,
  "is_media": false
}
```

`is_outgoing` is computed for the current user (`sender_id == viewer`).
A foreign or missing dialog returns `403`/`404` respectively.

For a media message `text` may be empty, `is_media` is `true`, `media_type` is the
stored MIME type, `media_size` is the file size in bytes and `media_name` is the
original file name (sanitized). The server-side storage path is never exposed.

### GET /api/chats/{id}

Chat details (used by the chat header and the "group info" modal):

```json
{
  "chat": { "id": 12, "title": "+7999...", "is_group": false, "last_message": null,
            "last_timestamp": null, "unread_count": 0, "other_user_id": 8, "is_online": false },
  "participants": [
    {"user_id": 7, "phone": "+79990000000", "is_admin": false, "is_online": true},
    {"user_id": 8, "phone": "+79990000001", "is_admin": false, "is_online": false}
  ],
  "is_admin": false
}
```

`is_admin` is the current user's role. Available to participants only (`403`).

### POST /api/chats/private

```json
{"phone": "+79990000001"}
```

Finds or creates a private dialog. `400` for an invalid phone number or a dialog with
yourself, `404` if the user does not exist. `200` — `ChatDetails`.

### POST /api/chats/group

```json
{"title": "Friends", "phones": ["+79990000001", "+79990000002"]}
```

The creator becomes the admin. `phones` is 0..=199 numbers (duplicates are ignored),
`title` is 1..=128 characters. Errors: `400` (validation), `404` (unknown number).
`200` — `ChatDetails`.

### PATCH /api/chats/{id}

```json
{"title": "New title"}
```

Group admin only, `title` 1..=128 characters. `200` — `ChatDetails`.

### POST /api/chats/{id}/participants

```json
{"phone": "+79990000003"}
```

Group admin only; the group is limited to 200 participants. `400/404/403`.
`200` — `ChatDetails`.

### DELETE /api/chats/{id}/participants/{user_id}

- an admin can remove any participant;
- any participant can remove themselves (leave the group);
- private dialogs cannot be modified (`400`).

`200 {"deleted": false}` if the group remains, `{"deleted": true}` if no participants are
left and the dialog has been deleted.

### POST /api/chats/{id}/read

Marks incoming messages of the dialog as read. Returns `200 {"marked": 3}`.
It also broadcasts a `messages_read` event to the dialog subscribers (see below).

### GET /api/users/lookup?phone=+79990000001

Exact user lookup by phone number (for "new chat" / "add participant").
Available to authenticated users, rate-limited. Returns
`200 {"user_id": 8, "phone": "+7999..."}` or `404 {"error":"user not found"}`.

## Media

### POST /api/chats/{id}/media?name=<file name>&caption=<optional text>

Uploads a file and creates a message in the dialog atomically.

- Headers: `Authorization: Bearer <token>`, `Content-Type: <MIME type>`;
  the request body is the raw file.
- `name` is the original file name (URL-encoded, ≤ 128 characters, path
  separators/control characters are stripped server-side).
- `caption` is optional message text (≤ 4096 characters; the same validation as
  `send_message`).
- Limits: file size up to `media.max_upload_mb` (default 25 MiB); allowed MIME
  types: `image/png`, `image/jpeg`, `image/webp`, `image/gif`, `application/pdf`,
  `text/plain`, `application/zip`, `video/mp4`. MIME types are validated; other
  types return `415`.
- Only dialog participants may upload (`403` otherwise, `404` for an unknown
  dialog). Rate-limited like the rest of the HTTP API.
- `200` response — the created `MessageItem` (see `/api/history`), with
  `is_media: true`. The server also broadcasts `new_message` to the dialog
  subscribers (`is_outgoing` is `false` in the broadcast; clients should compare
  `sender_id` with their own user id).
- Errors: `400` (bad name/caption), `403`, `404`, `413` (too large),
  `415` (unsupported type), `429`.

### GET /api/media/{message_id}

Returns the file bytes of a media message.

- Requires authentication and dialog participation (`401`/`403`/`404`).
- `200` with the stored `Content-Type`; images are served with
  `Content-Disposition: inline`, other files with `attachment` and a
  `filename*=UTF-8''...` name. Responses are marked `Cache-Control: private`,
  `X-Content-Type-Options: nosniff`.
- Only messages with `is_media: true` can be fetched; otherwise `404`.

The UI must fetch media with the `Authorization` header (not via a plain
`<img src>`), e.g. `fetch('/api/media/105')` and `URL.createObjectURL`.

## WebSocket `/ws`

The first frame is authentication. Format: `{"action": "...", "payload": {...}}`.

Client → server:
- `auth`: `{"token": "..."}` → `auth_success` / `auth_failed`
- `subscribe`: `{"dialog_id": 12}` → `subscribed` / `error`
- `send_message`: `{"dialog_id": 12, "text": "..."}` → `new_message` to the sender and
  to the other subscribers of the dialog
- `typing`: `{"dialog_id": 12, "typing": true|false}` — the server broadcasts `typing`
  to the other subscribers of the dialog (not echoed back to the sender). The client
  throttles these events (at most about once every 2 s).

Server → client:
- `connected` — sent immediately after the socket opens
- `auth_success {"user_id"}` / `auth_failed {"error"}`
- `subscribed {"dialog_id"}`
- `new_message` — a message object (same fields as in `/api/history`, plus
  `dialog_id`). `is_outgoing` is computed for the frame's recipient: the sender gets
  `true`, the other subscribers get `false`.
- `typing {"dialog_id", "user_id", "typing"}`
- `user_status {"user_id", "status"}` — `0` online, `1` offline; broadcast only to
  users who share a dialog
- `messages_read {"dialog_id", "reader_id"}` — someone read incoming messages
- `error {"error"}`

Limits: auth — 10 attempts/minute per IP, frames — 60/s per IP, messages — 10/s per
user, typing has its own limit; message text must be non-empty and at most 4096
characters.