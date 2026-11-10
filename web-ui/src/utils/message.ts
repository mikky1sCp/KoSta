import type { Message } from '../types';

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function asNumber(value: unknown, fallback = 0): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

function asNullableString(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

function asNullableNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

/**
 * Разбор `MessageItem` из docs/web-api.md (ответ истории, эхо `new_message`
 * и ответ `POST /api/chats/{id}/media`). Неизвестные/битые поля не ломают UI:
 * медиа-поля приводятся к `null`, флаги — к `false`.
 */
export function parseMessagePayload(value: unknown): Message | null {
  if (!isRecord(value)) {
    return null;
  }
  if (
    typeof value.id !== 'number' ||
    typeof value.sender_id !== 'number' ||
    typeof value.text !== 'string'
  ) {
    return null;
  }
  return {
    id: value.id,
    sender_id: value.sender_id,
    text: value.text,
    timestamp: asNumber(value.timestamp),
    is_outgoing: value.is_outgoing === true,
    read: value.read === true,
    delivered: value.delivered === true,
    media_name: asNullableString(value.media_name),
    media_type: asNullableString(value.media_type),
    media_size: asNullableNumber(value.media_size),
    is_media: value.is_media === true,
    reply_to_id: asNullableNumber(value.reply_to_id),
    reply_to_text: asNullableString(value.reply_to_text),
    reply_to_sender_id: asNullableNumber(value.reply_to_sender_id),
    reply_to_is_media: value.reply_to_is_media === true,
    edited_at: asNullableNumber(value.edited_at),
  };
}
