import type { Message, WsStatus } from './types';
import { parseMessagePayload } from './utils/message';

type Payload = Record<string, unknown>;

export type WsEvent =
  | { type: 'connected' }
  | { type: 'subscribed'; payload: { dialog_id: number } }
  | { type: 'new_message'; payload: Message & { dialog_id?: number } }
  | { type: 'typing'; payload: { dialog_id: number; user_id: number; typing: boolean } }
  | { type: 'user_status'; payload: { user_id: number; status: number } }
  | { type: 'messages_read'; payload: { dialog_id: number; reader_id: number } }
  | {
      type: 'message_edited';
      payload: { dialog_id: number; message_id: number; text: string; edited_at: number };
    }
  | { type: 'message_deleted'; payload: { dialog_id: number; message_id: number } }
  | { type: 'error'; payload: { error: string } };

export interface WsHandlers {
  onEvent: (event: WsEvent) => void;
  onStatus: (status: WsStatus) => void;
  /** Токен отвергнут сервером — приложение должно разлогиниться. */
  onAuthFailed: (error: string) => void;
}

const MAX_RECONNECT_DELAY_MS = 15_000;
const BASE_RECONNECT_DELAY_MS = 1_000;

/** URL WebSocket: в проде тот же origin, в dev — через прокси Vite. */
export function getWebSocketUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${protocol}//${window.location.host}/ws`;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

function asNumber(value: unknown, fallback = 0): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback;
}

function asString(value: unknown, fallback = ''): string {
  return typeof value === 'string' ? value : fallback;
}

function asNullableNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

/**
 * Медиа-сообщения приходят с теми же полями, что и в истории
 * (`media_name`, `media_type`, `media_size`, `is_media`), плюс `dialog_id`.
 */
function parseMessage(payload: Payload): (Message & { dialog_id?: number }) | null {
  const message = parseMessagePayload(payload);
  if (!message) {
    return null;
  }
  const dialogId = asNullableNumber(payload.dialog_id);
  return {
    ...message,
    ...(dialogId !== null ? { dialog_id: dialogId } : {}),
  };
}

/**
 * WebSocket-клиент KoSta.
 *
 * - первый кадр — `auth` с токеном (токен живёт в памяти);
 * - после авторизации клиент сам повторяет `subscribe` на последний диалог;
 * - при обрыве переподключается с экспоненциальным backoff, пока сессия жива
 *   (`close()` явно отключает переподключение — logout);
 * - возвращается к работе, когда вкладка снова видима или вернулась сеть.
 */
export class WsClient {
  private readonly url: string;
  private readonly token: string;
  private readonly handlers: WsHandlers;

  private socket: WebSocket | null = null;
  private authed = false;
  private closedByUser = false;
  private reconnectAttempt = 0;
  private reconnectTimer: number | null = null;
  private lastDialogId: number | null = null;
  private status: WsStatus = 'offline';

  constructor(options: { url: string; token: string; handlers: WsHandlers }) {
    this.url = options.url;
    this.token = options.token;
    this.handlers = options.handlers;
  }

  connect(): void {
    if (this.closedByUser) {
      return;
    }
    window.addEventListener('online', this.handleOnline);
    document.addEventListener('visibilitychange', this.handleVisibilityChange);
    this.openSocket();
  }

  /** Осознанное закрытие: logout или размонтирование приложения. */
  close(): void {
    this.closedByUser = true;
    window.removeEventListener('online', this.handleOnline);
    document.removeEventListener('visibilitychange', this.handleVisibilityChange);
    this.clearReconnectTimer();

    const socket = this.socket;
    this.socket = null;
    this.authed = false;
    if (socket) {
      socket.onopen = null;
      socket.onmessage = null;
      socket.onerror = null;
      socket.onclose = null;
      try {
        socket.close();
      } catch {
        /* сокет мог уже закрыться */
      }
    }
    this.setStatus('offline');
  }

  isConnected(): boolean {
    return this.authed && this.socket?.readyState === WebSocket.OPEN;
  }

  /** Запоминаем диалог и подписываемся, если соединение уже авторизовано. */
  subscribe(dialogId: number): void {
    this.lastDialogId = dialogId;
    if (this.isConnected()) {
      this.sendFrame('subscribe', { dialog_id: dialogId });
    }
  }

  sendMessage(dialogId: number, text: string, replyToId?: number | null): boolean {
    const payload: Payload = { dialog_id: dialogId, text };
    if (replyToId != null) {
      payload.reply_to_id = replyToId;
    }
    return this.sendFrame('send_message', payload);
  }

  sendTyping(dialogId: number, typing: boolean): boolean {
    return this.sendFrame('typing', { dialog_id: dialogId, typing });
  }

  private readonly handleOnline = (): void => {
    this.reconnectNow();
  };

  private readonly handleVisibilityChange = (): void => {
    if (document.visibilityState === 'visible') {
      this.reconnectNow();
    }
  };

  private openSocket(): void {
    if (this.closedByUser) {
      return;
    }
    if (
      this.socket &&
      (this.socket.readyState === WebSocket.OPEN ||
        this.socket.readyState === WebSocket.CONNECTING)
    ) {
      return;
    }

    this.setStatus(this.reconnectAttempt > 0 ? 'reconnecting' : 'connecting');

    let socket: WebSocket;
    try {
      socket = new WebSocket(this.url);
    } catch {
      this.scheduleReconnect();
      return;
    }
    this.socket = socket;

    socket.onopen = () => {
      this.sendAuth(socket);
    };

    socket.onmessage = (event: MessageEvent) => {
      if (typeof event.data === 'string') {
        this.handleFrame(event.data);
      }
    };

    socket.onerror = () => {
      // Ошибку не логируем: детали соединения есть в onclose/сервере.
    };

    socket.onclose = () => {
      this.handleSocketClosed(socket);
    };
  }

  private sendAuth(socket: WebSocket): void {
    if (socket.readyState !== WebSocket.OPEN) {
      return;
    }
    socket.send(JSON.stringify({ action: 'auth', payload: { token: this.token } }));
  }

  private handleFrame(raw: string): void {
    let frame: unknown;
    try {
      frame = JSON.parse(raw) as unknown;
    } catch {
      return;
    }
    if (!isRecord(frame) || typeof frame.type !== 'string') {
      return;
    }
    const payload: Payload = isRecord(frame.payload) ? frame.payload : {};

    switch (frame.type) {
      case 'connected':
        this.handlers.onEvent({ type: 'connected' });
        break;
      case 'auth_success':
        this.authed = true;
        this.reconnectAttempt = 0;
        this.setStatus('connected');
        this.resubscribe();
        break;
      case 'auth_failed':
        this.failAuthentication(
          asString(payload.error, 'Не удалось авторизовать соединение'),
        );
        break;
      case 'subscribed':
        this.handlers.onEvent({
          type: 'subscribed',
          payload: { dialog_id: asNumber(payload.dialog_id) },
        });
        break;
      case 'new_message': {
        const message = parseMessage(payload);
        if (message) {
          this.handlers.onEvent({ type: 'new_message', payload: message });
        }
        break;
      }
      case 'typing':
        this.handlers.onEvent({
          type: 'typing',
          payload: {
            dialog_id: asNumber(payload.dialog_id),
            user_id: asNumber(payload.user_id),
            typing: payload.typing === true,
          },
        });
        break;
      case 'user_status':
        this.handlers.onEvent({
          type: 'user_status',
          payload: {
            user_id: asNumber(payload.user_id),
            status: asNumber(payload.status, 1),
          },
        });
        break;
      case 'messages_read':
        this.handlers.onEvent({
          type: 'messages_read',
          payload: {
            dialog_id: asNumber(payload.dialog_id),
            reader_id: asNumber(payload.reader_id),
          },
        });
        break;
      case 'message_edited':
        this.handlers.onEvent({
          type: 'message_edited',
          payload: {
            dialog_id: asNumber(payload.dialog_id),
            message_id: asNumber(payload.message_id),
            text: asString(payload.text),
            edited_at: asNumber(payload.edited_at),
          },
        });
        break;
      case 'message_deleted':
        this.handlers.onEvent({
          type: 'message_deleted',
          payload: {
            dialog_id: asNumber(payload.dialog_id),
            message_id: asNumber(payload.message_id),
          },
        });
        break;
      case 'error':
        this.handlers.onEvent({
          type: 'error',
          payload: { error: asString(payload.error, 'Ошибка соединения') },
        });
        break;
      default:
        break;
    }
  }

  private handleSocketClosed(socket: WebSocket): void {
    if (this.socket !== socket) {
      return;
    }
    this.socket = null;
    this.authed = false;
    if (this.closedByUser) {
      this.setStatus('offline');
      return;
    }
    this.scheduleReconnect();
  }

  private scheduleReconnect(): void {
    if (this.closedByUser) {
      return;
    }
    this.setStatus('reconnecting');
    const exponent = Math.min(this.reconnectAttempt, 4);
    const delay =
      Math.min(BASE_RECONNECT_DELAY_MS * 2 ** exponent, MAX_RECONNECT_DELAY_MS) +
      Math.floor(Math.random() * 300);
    this.reconnectAttempt = Math.min(this.reconnectAttempt + 1, 10);

    this.clearReconnectTimer();
    this.reconnectTimer = window.setTimeout(() => {
      this.reconnectTimer = null;
      this.openSocket();
    }, delay);
  }

  private reconnectNow(): void {
    if (this.closedByUser || this.isConnected()) {
      return;
    }
    if (this.socket && this.socket.readyState === WebSocket.CONNECTING) {
      return;
    }
    this.clearReconnectTimer();
    this.openSocket();
  }

  private failAuthentication(error: string): void {
    this.closedByUser = true;
    this.authed = false;
    this.clearReconnectTimer();

    const socket = this.socket;
    this.socket = null;
    if (socket) {
      socket.onclose = null;
      try {
        socket.close();
      } catch {
        /* ignore */
      }
    }

    this.setStatus('offline');
    this.handlers.onAuthFailed(error);
  }

  private resubscribe(): void {
    if (this.lastDialogId !== null) {
      this.sendFrame('subscribe', { dialog_id: this.lastDialogId });
    }
  }

  private sendFrame(action: string, payload: Payload): boolean {
    const socket = this.socket;
    if (!socket || socket.readyState !== WebSocket.OPEN || !this.authed) {
      return false;
    }
    try {
      socket.send(JSON.stringify({ action, payload }));
      return true;
    } catch {
      return false;
    }
  }

  private clearReconnectTimer(): void {
    if (this.reconnectTimer !== null) {
      window.clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
  }

  private setStatus(status: WsStatus): void {
    if (this.status === status) {
      return;
    }
    this.status = status;
    this.handlers.onStatus(status);
  }
}
