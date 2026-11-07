import type {
  ChatDetails,
  ChatItem,
  LoginResponse,
  Me,
  Message,
  UserLookup,
} from './types';

/** Базовый путь: в проде и dev (через прокси Vite) — всегда свой origin. */
export const API_BASE = '/api';

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

    const rawBody = await response.text();
    let payload: unknown = null;
    if (rawBody.length > 0) {
      try {
        payload = JSON.parse(rawBody) as unknown;
      } catch {
        payload = null;
      }
    }

    if (!response.ok) {
      if (response.status === 401 && token) {
        onUnauthorized();
      }
      const serverMessage =
        isRecord(payload) && typeof payload.error === 'string' ? payload.error : '';
      throw new ApiError(
        serverMessage || `Ошибка сервера (${response.status})`,
        response.status,
      );
    }

    return payload as T;
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
  };
}

export type Api = ReturnType<typeof createApi>;
