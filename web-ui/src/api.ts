import type {
  ChatDetails,
  ChatItem,
  LoginResponse,
  Me,
  Message,
  UserLookup,
} from './types';
import { parseMessagePayload } from './utils/message';

/** Базовый путь: в проде и dev (через прокси Vite) — всегда свой origin. */
export const API_BASE = '/api';

/** Лимит размера вложения (совпадает с `media.max_upload_mb` по умолчанию). */
export const MEDIA_MAX_BYTES = 25 * 1024 * 1024;

/** MIME-типы, которые принимает сервер (`415` для остальных). */
export const MEDIA_ALLOWED_TYPES = [
  'image/png',
  'image/jpeg',
  'image/webp',
  'image/gif',
  'application/pdf',
  'text/plain',
  'application/zip',
  'video/mp4',
] as const;

export type MediaMimeType = (typeof MEDIA_ALLOWED_TYPES)[number];

/** Значение атрибута accept для <input type="file">. */
export const MEDIA_ACCEPT = [
  ...MEDIA_ALLOWED_TYPES,
  '.png',
  '.jpg',
  '.jpeg',
  '.webp',
  '.gif',
  '.pdf',
  '.txt',
  '.zip',
  '.mp4',
].join(',');

/** Максимальная длина `name` в query (сервер: ≤ 128 символов). */
const MEDIA_NAME_MAX_LENGTH = 128;

export function isAllowedMediaType(value: string): value is MediaMimeType {
  return (MEDIA_ALLOWED_TYPES as readonly string[]).includes(value);
}

export class ApiError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

interface RequestOptions extends Omit<RequestInit, 'headers'> {
  headers?: HeadersInit;
  /** Явный токен (нужен для logout, когда сессия уже очищена локально). */
  authToken?: string | null;
}

interface ApiOptions {
  /** Токен живёт только в памяти — забираем его из замыкания на каждом запросе. */
  getToken: () => string | null;
  /** 401 от сервера — сессия протухла, приложение должно выйти на экран входа. */
  onUnauthorized: () => void;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function parseJson(raw: string): unknown {
  if (raw.length === 0) {
    return null;
  }
  try {
    return JSON.parse(raw) as unknown;
  } catch {
    return null;
  }
}

function errorTextFromPayload(payload: unknown, status: number): string {
  const serverMessage =
    isRecord(payload) && typeof payload.error === 'string' ? payload.error : '';
  return serverMessage || `Ошибка сервера (${status})`;
}

/**
 * Имя файла для query: без управляющих переводов строк, непустое,
 * не длиннее 128 символов (остальное санитизирует сервер).
 */
function uploadFileName(name: string): string {
  const cleaned = name.replace(/[\u0000-\u001f\u007f]+/g, ' ').trim();
  const base = cleaned.length > 0 ? cleaned : 'file';
  return base.length > MEDIA_NAME_MAX_LENGTH
    ? base.slice(0, MEDIA_NAME_MAX_LENGTH)
    : base;
}

/**
 * Тонкий клиент REST API. Все ошибки приводит к ApiError с текстом сервера.
 * Токен не логируется и не попадает в URL.
 */
export function createApi({ getToken, onUnauthorized }: ApiOptions) {
  async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
    const { authToken, headers: customHeaders, ...init } = options;
    const token = authToken !== undefined ? authToken : getToken();

    const headers = new Headers(customHeaders);
    if (typeof init.body === 'string' && !headers.has('Content-Type')) {
      headers.set('Content-Type', 'application/json');
    }
    if (token) {
      headers.set('Authorization', `Bearer ${token}`);
    }

    let response: Response;
    try {
      response = await fetch(`${API_BASE}${path}`, { ...init, headers });
    } catch {
      throw new ApiError('Не удалось подключиться к серверу', 0);
    }

    const payload = parseJson(await response.text());

    if (!response.ok) {
      if (response.status === 401 && token) {
        onUnauthorized();
      }
      throw new ApiError(errorTextFromPayload(payload, response.status), response.status);
    }

    return payload as T;
  }

  /**
   * Загрузка файла: raw body + `Content-Type` файла, `name`/`caption` в query.
   * XMLHttpRequest нужен ради `upload.onprogress` (у fetch его нет).
   */
  function uploadMedia(
    chatId: number,
    file: File,
    caption: string,
    onProgress?: (percent: number) => void,
  ): Promise<Message> {
    const token = getToken();
    const fileName = uploadFileName(file.name);

    return new Promise<Message>((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      const path =
        `${API_BASE}/chats/${encodeURIComponent(String(chatId))}/media` +
        `?name=${encodeURIComponent(fileName)}` +
        `&caption=${encodeURIComponent(caption)}`;

      try {
        xhr.open('POST', path);
      } catch {
        reject(new ApiError('Не удалось начать загрузку файла', 0));
        return;
      }

      if (token) {
        xhr.setRequestHeader('Authorization', `Bearer ${token}`);
      }
      xhr.setRequestHeader('Content-Type', file.type || 'application/octet-stream');
      xhr.responseType = 'text';

      xhr.upload.onprogress = (event: ProgressEvent) => {
        if (!onProgress || !event.lengthComputable || event.total <= 0) {
          return;
        }
        const percent = Math.round((event.loaded / event.total) * 100);
        onProgress(Math.max(0, Math.min(100, percent)));
      };

      xhr.onerror = () => {
        reject(new ApiError('Не удалось подключиться к серверу', 0));
      };
      xhr.onabort = () => {
        reject(new ApiError('Загрузка файла прервана', 0));
      };
      xhr.ontimeout = () => {
        reject(new ApiError('Сервер не ответил вовремя', 0));
      };
      xhr.onload = () => {
        const raw = typeof xhr.responseText === 'string' ? xhr.responseText : '';
        const payload = parseJson(raw);

        if (xhr.status < 200 || xhr.status >= 300) {
          if (xhr.status === 401 && token) {
            onUnauthorized();
          }
          reject(new ApiError(errorTextFromPayload(payload, xhr.status), xhr.status));
          return;
        }

        const message = parseMessagePayload(payload);
        if (!message) {
          reject(new ApiError('Некорректный ответ сервера', xhr.status));
          return;
        }
        onProgress?.(100);
        resolve(message);
      };

      xhr.send(file);
    });
  }

  /** Файл медиа-сообщения: токен только в заголовке, тело — Blob. */
  async function fetchMediaBlob(messageId: number): Promise<Blob> {
    const token = getToken();
    const headers = new Headers();
    if (token) {
      headers.set('Authorization', `Bearer ${token}`);
    }

    let response: Response;
    try {
      response = await fetch(
        `${API_BASE}/media/${encodeURIComponent(String(messageId))}`,
        { headers },
      );
    } catch {
      throw new ApiError('Не удалось подключиться к серверу', 0);
    }

    if (!response.ok) {
      if (response.status === 401 && token) {
        onUnauthorized();
      }
      let raw = '';
      try {
        raw = await response.text();
      } catch {
        raw = '';
      }
      throw new ApiError(
        errorTextFromPayload(parseJson(raw), response.status),
        response.status,
      );
    }

    try {
      return await response.blob();
    } catch {
      throw new ApiError('Не удалось прочитать файл', response.status);
    }
  }

  return {
    login(phone: string, password: string, register: boolean): Promise<LoginResponse> {
      return request<LoginResponse>('/login', {
        method: 'POST',
        body: JSON.stringify({ phone, password, register }),
      });
    },

    logout(token: string): Promise<{ success: boolean }> {
      return request<{ success: boolean }>('/logout', {
        method: 'POST',
        authToken: token,
      });
    },

    me(): Promise<Me> {
      return request<Me>('/me');
    },

    listChats(): Promise<ChatItem[]> {
      return request<ChatItem[]>('/chats');
    },

    history(chatId: number, limit: number, offset: number): Promise<Message[]> {
      return request<Message[]>(
        `/history?chat_id=${encodeURIComponent(String(chatId))}` +
          `&limit=${encodeURIComponent(String(limit))}` +
          `&offset=${encodeURIComponent(String(offset))}`,
      );
    },

    chatDetails(chatId: number): Promise<ChatDetails> {
      return request<ChatDetails>(`/chats/${encodeURIComponent(String(chatId))}`);
    },

    /** Правка своего сообщения; остальные подписчики получают WS-событие. */
    editMessage(messageId: number, text: string): Promise<unknown> {
      return request<unknown>(`/messages/${encodeURIComponent(String(messageId))}`, {
        method: 'PATCH',
        body: JSON.stringify({ text }),
      });
    },

    /** Удаление своего сообщения (мягкое: из истории пропадает). */
    deleteMessage(messageId: number): Promise<unknown> {
      return request<unknown>(`/messages/${encodeURIComponent(String(messageId))}`, {
        method: 'DELETE',
      });
    },

    createPrivate(phone: string): Promise<ChatDetails> {
      return request<ChatDetails>('/chats/private', {
        method: 'POST',
        body: JSON.stringify({ phone }),
      });
    },

    createGroup(title: string, phones: string[]): Promise<ChatDetails> {
      return request<ChatDetails>('/chats/group', {
        method: 'POST',
        body: JSON.stringify({ title, phones }),
      });
    },

    renameChat(chatId: number, title: string): Promise<ChatDetails> {
      return request<ChatDetails>(`/chats/${encodeURIComponent(String(chatId))}`, {
        method: 'PATCH',
        body: JSON.stringify({ title }),
      });
    },

    addParticipant(chatId: number, phone: string): Promise<ChatDetails> {
      return request<ChatDetails>(
        `/chats/${encodeURIComponent(String(chatId))}/participants`,
        {
          method: 'POST',
          body: JSON.stringify({ phone }),
        },
      );
    },

    removeParticipant(chatId: number, userId: number): Promise<{ deleted: boolean }> {
      return request<{ deleted: boolean }>(
        `/chats/${encodeURIComponent(String(chatId))}/participants/${encodeURIComponent(
          String(userId),
        )}`,
        { method: 'DELETE' },
      );
    },

    markRead(chatId: number): Promise<{ marked: number }> {
      return request<{ marked: number }>(
        `/chats/${encodeURIComponent(String(chatId))}/read`,
        { method: 'POST' },
      );
    },

    lookupUser(phone: string): Promise<UserLookup> {
      return request<UserLookup>(
        `/users/lookup?phone=${encodeURIComponent(phone)}`,
      );
    },

    uploadMedia,
    fetchMediaBlob,
  };
}

export type Api = ReturnType<typeof createApi>;
