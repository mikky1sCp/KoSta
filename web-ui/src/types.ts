// Типы API/WS из docs/web-api.md. Все поля — snake_case, как в контракте.

export interface LoginResponse {
  success: boolean;
  token: string | null;
  auth_key_id: number | null;
  user_id: number | null;
  error: string | null;
}

export interface Me {
  user_id: number;
  phone: string;
  is_online: boolean;
}

export interface ChatItem {
  id: number;
  title: string;
  is_group: boolean;
  last_message: string | null;
  last_timestamp: number | null;
  unread_count: number;
  /** Только для личных чатов. */
  other_user_id?: number | null;
  /** Только для личных чатов. */
  is_online?: boolean | null;
}

export interface Message {
  id: number;
  sender_id: number;
  /** Для медиа может быть пустым (файл без подписи). */
  text: string;
  timestamp: number;
  is_outgoing: boolean;
  read: boolean;
  delivered: boolean;
  /** Оригинальное имя файла (санитизировано сервером). */
  media_name: string | null;
  /** MIME-тип файла. */
  media_type: string | null;
  /** Размер файла в байтах. */
  media_size: number | null;
  is_media: boolean;
  /** Сообщение, на которое это отвечает (цитата). */
  reply_to_id: number | null;
  /** Текст цитируемого сообщения; null, если оно удалено. */
  reply_to_text: string | null;
  /** Автор цитируемого сообщения. */
  reply_to_sender_id: number | null;
  /** Было ли цитируемое сообщение медиа. */
  reply_to_is_media: boolean;
  /** Время последней правки (unix seconds), null — не редактировалось. */
  edited_at: number | null;
}

/** Сообщение в локальном состоянии: pending — ещё летит на сервер. */
export interface UiMessage extends Message {
  pending?: boolean;
}

export interface Participant {
  user_id: number;
  phone: string;
  is_admin: boolean;
  is_online: boolean;
}

export interface ChatDetails {
  chat: ChatItem;
  participants: Participant[];
  is_admin: boolean;
}

export interface UserLookup {
  user_id: number;
  phone: string;
}

export type Theme = 'dark' | 'light';

export type WsStatus = 'offline' | 'connecting' | 'connected' | 'reconnecting';

export type ToastKind = 'error' | 'success' | 'info';
