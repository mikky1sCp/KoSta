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
- Rate limits: `429` when the per-IP login limit (10/min) or the per-account
  limit (10 attempts per 15 minutes for the same phone number) is exceeded.
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
{
  "user_id": 7,
  "phone": "+79990000000",
  "display_name": "Аня",
  "has_avatar": true,
  "is_online": true
}
```

`display_name` is `null` until the user sets one; clients then show the phone.
`has_avatar` tells whether `GET /api/avatars/{user_id}` returns an image.

The web client stores the token in memory by default and, when the user enables
"Запомнить меня" (on by default at login), also in `localStorage` under
`kosta.session.v1` with a 7-day expiry that mirrors the server-side token TTL.
Logging out, or any `401`, clears that entry.

### PATCH /api/me

Update your own profile. Body: `{"display_name": "Аня"}` (up to 64 characters;
an empty or blank string resets the name back to the phone number).

- `200` — the updated `MeResponse` (same shape as `GET /api/me`).
- `400` — the name is too long; `401` — no valid token.

### POST /api/me/avatar

Upload a profile picture. The raw image is the request body and its type comes from
`Content-Type`; only `image/png`, `image/jpeg` and `image/webp` are accepted, up to
5 MiB. The declared type is checked against the file signature (PNG/JPEG/WebP magic
bytes), so a file with spoofed `Content-Type` is rejected. Replacing the avatar
deletes the previous file.

- `200` — `{"ok": true, "has_avatar": true}`
- `400` — empty body; `415` — unsupported type or bytes that do not match it;
  `413` — too large.

### GET /api/avatars/{user_id}

The user's avatar bytes (`image/png`, `image/jpeg` or `image/webp`), or `404` when
the user has no avatar. No token is required: the image is used directly in `<img>`
tags, and browsers do not send `Authorization` for them. Avatars are visible to
everyone, which matches how they are shown to chat partners.

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
  "is_media": false,
  "reply_to_id": null,
  "reply_to_text": null,
  "reply_to_sender_id": null,
  "reply_to_is_media": false,
  "edited_at": null,
  "reactions": []
}
```

`is_outgoing` is computed for the current user (`sender_id == viewer`).
A foreign or missing dialog returns `403`/`404` respectively.

For a media message `text` may be empty, `is_media` is `true`, `media_type` is the
stored MIME type, `media_size` is the file size in bytes and `media_name` is the
original file name (sanitized). The server-side storage path is never exposed.

A message that replies to another one carries the quoted message in `reply_to_*`:
`reply_to_id` is the quoted message, `reply_to_text` its text (or `null` when it was
deleted), `reply_to_sender_id` its author and `reply_to_is_media` tells whether the
quoted message was an attachment. `edited_at` is the last edit time in unix seconds,
or `null` if the message was never edited.

### GET /api/search?chat_id=12&q=привет&limit=50

Search the dialog's messages by text, case-insensitively (Cyrillic included) and
ignoring deleted messages. Returns an array of `MessageItem` (same shape as
`/api/history`), newest first; `limit` is clamped to 1..=200 and defaults to 50.

The comparison uses a normalized `text_lower` column maintained on insert and
edit, because SQLite's `LIKE` is case-insensitive only for ASCII. `%` and `_` in
the query are escaped, so "100%" matches literally.

- `400` — empty or longer than 128 characters query;
  `403` — not a participant; `401` — no valid token.

### POST /api/messages/{id}/reactions

Toggle a reaction on a message. Body: `{"emoji": "👍"}` (up to 8 characters, no
control characters). Calling it again with the same emoji removes the reaction.

```json
{
  "ok": true,
  "reactions": [{"emoji": "👍", "count": 2, "me": true}]
}
```

`reactions` is grouped per emoji, `count` is the number of users and `me` says
whether the caller reacted. Subscribers of the dialog receive a
`message_reactions` event with the same grouping but without `me`, since that flag
is viewer-specific.

- `400` — empty or too long emoji; `403` — not a participant;
  `404` — message not found or deleted.

### PATCH /api/messages/{id}

Edit one of your own messages. Body: `{"text": "new text"}` (1..4096 characters).

- `200` — `{"ok": true, "edited_at": 1794057600}`; subscribers of the dialog receive a
  `message_edited` WebSocket event.
- `400` — empty or too long text; `403` — not the sender or not a participant;
  `404` — message not found or already deleted.

### DELETE /api/messages/{id}

Delete one of your own messages (soft delete: the row stays in the database with
`deleted = 1` and history no longer returns it).

- `200` — `{"ok": true}`; subscribers receive a `message_deleted` WebSocket event.
- `403` — not the sender or not a participant; `404` — message not found.

To reply, send `send_message` over the WebSocket with an extra `reply_to_id` field:
`{"action":"send_message","payload":{"dialog_id":12,"text":"ok","reply_to_id":105}}`.
The target message must belong to the same dialog and not be deleted, otherwise the
server answers with an `error` event.

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
  "is_admin": false,
  "pinned_message": null
}
```

`pinned_message` is the currently pinned `MessageItem` of the dialog (same shape as in
`/api/history`), or `null` when nothing is pinned or the pinned message was deleted.

### POST /api/chats/{id}/pin

Pin a message or clear the pin. Body: `{"message_id": 105}`, or
`{"message_id": null}` to unpin.

- Any participant may pin in a private chat; in a group only an admin may (`403`).
- The message must belong to this dialog and not be deleted (`404` otherwise).
- `200` — `{"ok": true, "pinned_message": MessageItem | null}`; subscribers receive a
  `dialog_pinned` event with the same object.
- Deleting a pinned message automatically clears the pin and broadcasts
  `dialog_pinned {"message": null}`.

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
  `text/plain`, `application/zip`, `video/mp4`, plus audio for voice messages:
  `audio/webm`, `audio/ogg`, `audio/mpeg`, `audio/mp4`, `audio/wav`. MIME types are
  validated; other types return `415`. The declared `Content-Type` must also match
  the file signature (magic bytes): e.g. PNG bytes sent as `text/plain` are rejected
  with `415`, and `text/plain` must be valid UTF-8.
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
- `message_edited {"dialog_id", "message_id", "text", "edited_at"}` — a message was
  edited by its author
- `message_deleted {"dialog_id", "message_id"}` — a message was deleted by its author
- `message_reactions {"dialog_id", "message_id", "reactions": [{"emoji", "count"}]}` —
  reactions on a message changed; `me` is intentionally absent, each client keeps
  its own flag
- `dialog_pinned {"dialog_id", "message": MessageItem | null}` — the pinned message
  changed; `null` means the pin was cleared
- `error {"error"}`

Limits: auth — 10 attempts/minute per IP, frames — 60/s per IP, messages — 10/s per
user, typing has its own limit; message text must be non-empty and at most 4096
characters.