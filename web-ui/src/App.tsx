import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ApiError, createApi, type Api } from './api';
import { WsClient, getWebSocketUrl, type WsEvent } from './ws';
import type {
  ChatDetails,
  ChatItem,
  Me,
  Message,
  UiMessage,
  WsStatus,
} from './types';
import { getErrorMessage } from './utils/errors';
import { buildTypingLabel } from './utils/format';
import { downloadMedia, messagePreview } from './utils/media';
import { useTheme } from './hooks/useTheme';
import { useToasts } from './hooks/useToasts';
import { useDrafts } from './hooks/useDrafts';
import { AuthScreen } from './components/AuthScreen';
import { Sidebar } from './components/Sidebar';
import { ChatPanel } from './components/ChatPanel';
import { ToastViewport } from './components/ui/ToastViewport';
import { Lightbox } from './components/ui/Lightbox';
import { NewChatModal } from './components/modals/NewChatModal';
import { NewGroupModal } from './components/modals/NewGroupModal';
import { ChatInfoModal } from './components/modals/ChatInfoModal';
import { ProfileModal } from './components/modals/ProfileModal';

const PAGE_SIZE = 50;
const TYPING_TIMEOUT_MS = 3_000;
const TYPING_THROTTLE_MS = 2_000;
const TYPING_IDLE_MS = 2_000;
const READ_DEBOUNCE_MS = 250;

interface Session {
  token: string;
  userId: number;
  phone: string;
}

type ModalKind = 'new-chat' | 'new-group' | 'info' | 'profile';

/** Лайтбокс показывает уже скачанный blob-url сообщения из ленты. */
interface LightboxState {
  message: UiMessage;
  url: string;
}

function sortChats(chats: ChatItem[]): ChatItem[] {
  return [...chats].sort(
    (left, right) => (right.last_timestamp ?? 0) - (left.last_timestamp ?? 0),
  );
}

function toUiMessage(message: Message): UiMessage {
  return { ...message, pending: false };
}

/**
 * История приходит «новые сверху», но у сообщений одного второго timestamp
 * порядок в SQL может быть произвольным. Для отображения сортируем сами:
 * по времени, затем по id (стабильно и без дублей между страницами).
 */
function sortMessagesAscending(messages: Message[]): Message[] {
  return [...messages].sort(
    (left, right) => left.timestamp - right.timestamp || left.id - right.id,
  );
}

/**
 * Склеиваем историю с сообщениями, пришедшими по WS, пока она грузилась:
 * событие может опередить ответ истории, терять такие сообщения нельзя.
 */
function mergeHistory(
  history: Message[],
  current: UiMessage[],
): UiMessage[] {
  const byId = new Map<number, UiMessage>();
  for (const message of sortMessagesAscending(history)) {
    byId.set(message.id, toUiMessage(message));
  }
  for (const message of current) {
    byId.set(message.id, message);
  }
  return [...byId.values()].sort(
    (left, right) => left.timestamp - right.timestamp || left.id - right.id,
  );
}

function bumpChat(
  chats: ChatItem[],
  chatId: number,
  lastMessage: string,
  lastTimestamp: number,
): ChatItem[] {
  const index = chats.findIndex((chat) => chat.id === chatId);
  if (index === -1) {
    return chats;
  }
  const next = [...chats];
  next[index] = {
    ...next[index],
    last_message: lastMessage,
    last_timestamp: lastTimestamp,
  };
  return sortChats(next);
}

function applyIncomingMessage(
  chats: ChatItem[],
  dialogId: number,
  message: UiMessage,
  isActive: boolean,
): ChatItem[] {
  const index = chats.findIndex((chat) => chat.id === dialogId);
  if (index === -1) {
    return chats;
  }
  const current = chats[index];
  const next = [...chats];
  next[index] = {
    ...current,
    last_message: messagePreview(message),
    last_timestamp: message.timestamp,
    unread_count: isActive
      ? 0
      : message.is_outgoing
        ? current.unread_count
        : current.unread_count + 1,
  };
  return sortChats(next);
}

/** Оптимистичное сообщение заменяется серверным по эху new_message. */
function appendIncomingMessage(
  messages: UiMessage[],
  incoming: UiMessage,
): UiMessage[] {
  if (messages.some((message) => message.id === incoming.id)) {
    return messages;
  }
  if (incoming.is_outgoing) {
    let pendingIndex = -1;
    for (let index = 0; index < messages.length; index += 1) {
      const candidate = messages[index];
      if (
        candidate.pending &&
        candidate.text === incoming.text &&
        candidate.sender_id === incoming.sender_id
      ) {
        pendingIndex = index;
        break;
      }
    }
    if (pendingIndex !== -1) {
      const next = [...messages];
      next[pendingIndex] = incoming;
      return next;
    }
  }
  return [...messages, incoming];
}

export function App() {
  const [theme, setTheme, toggleTheme] = useTheme();
  const { toasts, remove, toast } = useToasts();
  const drafts = useDrafts();

  const [session, setSession] = useState<Session | null>(null);
  const [me, setMe] = useState<Me | null>(null);
  const [chats, setChats] = useState<ChatItem[]>([]);
  const [chatsLoading, setChatsLoading] = useState(false);
  const [activeChatId, setActiveChatId] = useState<number | null>(null);
  const [details, setDetails] = useState<ChatDetails | null>(null);
  const [messages, setMessages] = useState<UiMessage[]>([]);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [typingUsers, setTypingUsers] = useState<number[]>([]);
  const [wsStatus, setWsStatus] = useState<WsStatus>('offline');
  const [authLoading, setAuthLoading] = useState(false);
  const [authError, setAuthError] = useState('');
  const [modal, setModal] = useState<ModalKind | null>(null);
  const [lightbox, setLightbox] = useState<LightboxState | null>(null);
  const [unreadDividerMessageId, setUnreadDividerMessageId] = useState<
    number | null
  >(null);

  const sessionRef = useRef<Session | null>(null);
  const activeChatIdRef = useRef<number | null>(null);
  const meRef = useRef<Me | null>(null);
  const chatsRef = useRef<ChatItem[]>([]);
  const messagesRef = useRef<UiMessage[]>([]);
  const wsRef = useRef<WsClient | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const typingTimersRef = useRef(new Map<number, number>());
  const typingStopTimerRef = useRef<number | null>(null);
  const typingSentRef = useRef(false);
  const typingChatIdRef = useRef<number | null>(null);
  const lastTypingSentAtRef = useRef(0);
  const readTimerRef = useRef<number | null>(null);
  const loadingMoreRef = useRef(false);
  const pendingMessageIdRef = useRef(0);
  const onUnauthorizedRef = useRef<() => void>(() => {});
  const wsEventRef = useRef<(event: WsEvent) => void>(() => {});

  const api = useMemo<Api>(
    () =>
      createApi({
        getToken: () => sessionRef.current?.token ?? null,
        onUnauthorized: () => onUnauthorizedRef.current(),
      }),
    [],
  );

  // ---------- typing ----------

  const clearTyping = useCallback(() => {
    typingTimersRef.current.forEach((timer) => window.clearTimeout(timer));
    typingTimersRef.current.clear();
    setTypingUsers((current) => (current.length === 0 ? current : []));
  }, []);

  const stopTyping = useCallback(() => {
    if (typingStopTimerRef.current !== null) {
      window.clearTimeout(typingStopTimerRef.current);
      typingStopTimerRef.current = null;
    }
    const chatId = typingChatIdRef.current;
    typingChatIdRef.current = null;
    if (!typingSentRef.current) {
      return;
    }
    typingSentRef.current = false;
    if (chatId !== null) {
      wsRef.current?.sendTyping(chatId, false);
    }
  }, []);

  const noteTyping = useCallback((userId: number, typing: boolean) => {
    setTypingUsers((current) => {
      if (typing) {
        return current.includes(userId) ? current : [...current, userId];
      }
      return current.filter((id) => id !== userId);
    });

    const existing = typingTimersRef.current.get(userId);
    if (existing !== undefined) {
      window.clearTimeout(existing);
      typingTimersRef.current.delete(userId);
    }
    if (typing) {
      const timer = window.setTimeout(() => {
        typingTimersRef.current.delete(userId);
        setTypingUsers((current) => current.filter((id) => id !== userId));
      }, TYPING_TIMEOUT_MS);
      typingTimersRef.current.set(userId, timer);
    }
  }, []);

  const handleTypingInput = useCallback(
    (hasText: boolean) => {
      const chatId = activeChatIdRef.current;
      const client = wsRef.current;
      if (chatId === null || !client?.isConnected()) {
        return;
      }
      if (!hasText) {
        stopTyping();
        return;
      }
      if (typingStopTimerRef.current !== null) {
        window.clearTimeout(typingStopTimerRef.current);
        typingStopTimerRef.current = null;
      }
      const now = Date.now();
      if (
        !typingSentRef.current ||
        now - lastTypingSentAtRef.current >= TYPING_THROTTLE_MS
      ) {
        if (client.sendTyping(chatId, true)) {
          typingSentRef.current = true;
          typingChatIdRef.current = chatId;
          lastTypingSentAtRef.current = now;
        }
      }
      typingStopTimerRef.current = window.setTimeout(() => {
        typingStopTimerRef.current = null;
        stopTyping();
      }, TYPING_IDLE_MS);
    },
    [stopTyping],
  );

  // ---------- чтение и чаты ----------

  const markChatRead = useCallback(
    (chatId: number) => {
      if (!sessionRef.current) {
        return;
      }
      setChats((current) =>
        current.map((chat) =>
          chat.id === chatId && chat.unread_count !== 0
            ? { ...chat, unread_count: 0 }
            : chat,
        ),
      );
      void api.markRead(chatId).catch(() => {
        /* отметку прочтения можно повторить при следующем сообщении */
      });
    },
    [api],
  );

  const scheduleMarkRead = useCallback(
    (chatId: number) => {
      if (readTimerRef.current !== null) {
        window.clearTimeout(readTimerRef.current);
      }
      readTimerRef.current = window.setTimeout(() => {
        readTimerRef.current = null;
        markChatRead(chatId);
      }, READ_DEBOUNCE_MS);
    },
    [markChatRead],
  );

  const refreshChats = useCallback(
    async (options?: { silent?: boolean }) => {
      if (!sessionRef.current) {
        return;
      }
      if (!options?.silent) {
        setChatsLoading(true);
      }
      try {
        const list = await api.listChats();
        setChats(sortChats(list));
      } catch (error) {
        if (!(error instanceof ApiError && error.status === 401)) {
          toast.error(getErrorMessage(error));
        }
      } finally {
        if (!options?.silent) {
          setChatsLoading(false);
        }
      }
    },
    [api, toast],
  );

  // ---------- сессия ----------

  const resetSessionState = useCallback(() => {
    setMe(null);
    setChats([]);
    setActiveChatId(null);
    setDetails(null);
    setMessages([]);
    setHasMore(false);
    setHistoryLoading(false);
    setLoadingMore(false);
    setChatsLoading(false);
    setTypingUsers([]);
    setModal(null);
    setLightbox(null);
    setUnreadDividerMessageId(null);
    loadingMoreRef.current = false;
    clearTyping();
    stopTyping();
  }, [clearTyping, stopTyping]);

  const handleSessionExpired = useCallback(() => {
    if (!sessionRef.current) {
      return;
    }
    sessionRef.current = null;
    setSession(null);
    wsRef.current?.close();
    resetSessionState();
    toast.error('Сессия истекла. Войдите снова.');
  }, [resetSessionState, toast]);

  const handleAuth = useCallback(
    async (phone: string, password: string, register: boolean) => {
      setAuthLoading(true);
      setAuthError('');
      try {
        const response = await api.login(phone, password, register);
        if (!response.success || !response.token || response.user_id == null) {
          setAuthError(response.error ?? 'Не удалось выполнить вход');
          return;
        }
        const nextSession: Session = {
          token: response.token,
          userId: response.user_id,
          phone,
        };
        sessionRef.current = nextSession;
        setSession(nextSession);
        setMe({ user_id: response.user_id, phone, is_online: true });
        toast.success(
          register ? 'Аккаунт создан. Добро пожаловать!' : 'С возвращением!',
        );
      } catch (error) {
        setAuthError(getErrorMessage(error));
      } finally {
        setAuthLoading(false);
      }
    },
    [api, toast],
  );

  const handleLogout = useCallback(() => {
    const current = sessionRef.current;
    sessionRef.current = null;
    setSession(null);
    wsRef.current?.close();
    resetSessionState();
    if (current) {
      void api.logout(current.token).catch(() => {
        /* даже если сервер недоступен, локально мы уже вышли */
      });
    }
    toast.success('Вы вышли из аккаунта');
  }, [api, resetSessionState, toast]);

  // ---------- сообщения ----------

  const handleSend = useCallback(
    (rawText: string): boolean => {
      const text = rawText.trim();
      if (!text) {
        return false;
      }
      const chatId = activeChatIdRef.current;
      if (chatId === null) {
        return false;
      }
      const client = wsRef.current;
      if (!client?.isConnected()) {
        toast.error('Нет соединения с сервером — сообщение не отправлено');
        return false;
      }

      pendingMessageIdRef.current += 1;
      const timestamp = Math.floor(Date.now() / 1000);
      const pending: UiMessage = {
        id: -pendingMessageIdRef.current,
        sender_id: meRef.current?.user_id ?? 0,
        text,
        timestamp,
        is_outgoing: true,
        read: false,
        delivered: false,
        media_name: null,
        media_type: null,
        media_size: null,
        is_media: false,
        pending: true,
      };

      if (!client.sendMessage(chatId, text)) {
        toast.error('Не удалось отправить сообщение');
        return false;
      }
      stopTyping();
      setMessages((current) => [...current, pending]);
      setChats((current) => bumpChat(current, chatId, text, timestamp));
      return true;
    },
    [stopTyping, toast],
  );

  /**
   * Вложение: сервер создаёт сообщение сам (POST /api/chats/{id}/media).
   * Добавляем его в ленту без оптимистичной заглушки; WS-эхо с тем же id
   * потом отфильтруется при дедупликации в appendIncomingMessage.
   */
  const handleUpload = useCallback(
    async (
      file: File,
      caption: string,
      onProgress: (percent: number) => void,
    ): Promise<boolean> => {
      const chatId = activeChatIdRef.current;
      if (chatId === null) {
        return false;
      }
      try {
        const created = await api.uploadMedia(chatId, file, caption, onProgress);
        const myUserId = meRef.current?.user_id ?? null;
        const uiMessage: UiMessage = {
          ...created,
          // Сервер шлёт `is_outgoing: false` в broadcast — ориентируемся на sender_id.
          is_outgoing:
            myUserId !== null ? created.sender_id === myUserId : created.is_outgoing,
          pending: false,
        };
        const isActive = chatId === activeChatIdRef.current;
        if (isActive) {
          setMessages((current) => appendIncomingMessage(current, uiMessage));
        }
        if (!chatsRef.current.some((chat) => chat.id === chatId)) {
          void refreshChats({ silent: true });
        } else {
          setChats((current) =>
            applyIncomingMessage(current, chatId, uiMessage, isActive),
          );
        }
        return true;
      } catch (error) {
        if (!(error instanceof ApiError && error.status === 401)) {
          toast.error(getErrorMessage(error));
        }
        return false;
      }
    },
    [api, refreshChats, toast],
  );

  const handleMediaDownload = useCallback(
    async (message: UiMessage): Promise<void> => {
      try {
        await downloadMedia(api, message);
      } catch (error) {
        if (!(error instanceof ApiError && error.status === 401)) {
          toast.error(getErrorMessage(error));
        }
      }
    },
    [api, toast],
  );

  const handleCopyMessage = useCallback(
    (message: UiMessage) => {
      const text = message.text.trim();
      if (!text) {
        return;
      }
      void navigator.clipboard
        .writeText(text)
        .then(() => toast.success('Текст скопирован'))
        .catch(() => toast.error('Не удалось скопировать текст'));
    },
    [toast],
  );

  const handleOpenImage = useCallback((message: UiMessage, url: string) => {
    setLightbox({ message, url });
  }, []);

  const closeLightbox = useCallback(() => {
    setLightbox(null);
  }, []);

  const handleLoadMore = useCallback(async (): Promise<boolean> => {
    const chatId = activeChatIdRef.current;
    if (chatId === null || !hasMore || loadingMoreRef.current) {
      return false;
    }
    loadingMoreRef.current = true;
    setLoadingMore(true);
    try {
      const offset = messagesRef.current.filter((message) => !message.pending).length;
      const page = await api.history(chatId, PAGE_SIZE, offset);
      if (activeChatIdRef.current !== chatId) {
        return false;
      }
      const existingIds = new Set(messagesRef.current.map((message) => message.id));
      const older = sortMessagesAscending(page)
        .filter((message) => !existingIds.has(message.id))
        .map(toUiMessage);
      if (older.length === 0) {
        setHasMore(false);
        return false;
      }
      setMessages((current) => [...older, ...current]);
      setHasMore(page.length >= PAGE_SIZE);
      return true;
    } catch (error) {
      if (!(error instanceof ApiError && error.status === 401)) {
        toast.error(getErrorMessage(error));
      }
      return false;
    } finally {
      loadingMoreRef.current = false;
      setLoadingMore(false);
    }
  }, [api, hasMore, toast]);

  // ---------- WebSocket --------

  const handleWsEvent = (event: WsEvent) => {
    switch (event.type) {
      case 'new_message': {
        const incoming = event.payload;
        if (!Number.isFinite(incoming.id) || incoming.id <= 0) {
          return;
        }
        const myUserId = meRef.current?.user_id ?? null;
        const dialogId = incoming.dialog_id ?? activeChatIdRef.current;
        if (dialogId === null) {
          return;
        }
        const isOutgoing =
          myUserId !== null ? incoming.sender_id === myUserId : incoming.is_outgoing;
        const uiMessage: UiMessage = {
          ...incoming,
          is_outgoing: isOutgoing,
          pending: false,
        };
        const isActive = dialogId === activeChatIdRef.current;

        if (isActive) {
          setMessages((current) => appendIncomingMessage(current, uiMessage));
          if (!isOutgoing) {
            scheduleMarkRead(dialogId);
          }
        }

        if (!chatsRef.current.some((chat) => chat.id === dialogId)) {
          void refreshChats({ silent: true });
        } else {
          setChats((current) =>
            applyIncomingMessage(current, dialogId, uiMessage, isActive),
          );
        }
        break;
      }

      case 'typing': {
        const { dialog_id, user_id, typing } = event.payload;
        if (dialog_id !== activeChatIdRef.current) {
          return;
        }
        if (user_id === meRef.current?.user_id) {
          return;
        }
        noteTyping(user_id, typing);
        break;
      }

      case 'user_status': {
        const { user_id, status } = event.payload;
        const online = status === 0;
        setChats((current) =>
          current.map((chat) =>
            chat.other_user_id === user_id ? { ...chat, is_online: online } : chat,
          ),
        );
        setDetails((current) =>
          current
            ? {
                ...current,
                participants: current.participants.map((participant) =>
                  participant.user_id === user_id
                    ? { ...participant, is_online: online }
                    : participant,
                ),
              }
            : current,
        );
        break;
      }

      case 'messages_read': {
        const { dialog_id, reader_id } = event.payload;
        if (
          dialog_id === activeChatIdRef.current &&
          reader_id !== meRef.current?.user_id
        ) {
          setMessages((current) =>
            current.map((message) =>
              message.is_outgoing && !message.read
                ? { ...message, read: true, delivered: true }
                : message,
            ),
          );
        }
        break;
      }

      case 'error':
        toast.error(event.payload.error || 'Ошибка соединения');
        break;

      case 'connected':
      case 'subscribed':
        break;
    }
  };

  // ---------- создание/изменение чатов ----------

  const handleChatCreated = useCallback(
    (next: ChatDetails) => {
      setModal(null);
      setDetails(next);
      setActiveChatId(next.chat.id);
      setChats((current) => {
        const withoutChat = current.filter((chat) => chat.id !== next.chat.id);
        return sortChats([next.chat, ...withoutChat]);
      });
      void refreshChats({ silent: true });
      toast.success(next.chat.is_group ? 'Группа создана' : 'Чат создан');
    },
    [refreshChats, toast],
  );

  const handleDetailsUpdated = useCallback((next: ChatDetails) => {
    setDetails(next);
    setChats((current) =>
      current.map((chat) =>
        chat.id === next.chat.id
          ? { ...chat, title: next.chat.title, is_group: next.chat.is_group }
          : chat,
      ),
    );
  }, []);

  const handleLeftChat = useCallback(
    (chatId: number) => {
      setModal(null);
      setChats((current) => current.filter((chat) => chat.id !== chatId));
      setActiveChatId((current) => (current === chatId ? null : current));
      void refreshChats({ silent: true });
    },
    [refreshChats],
  );

  const handleSelectChat = useCallback((chatId: number) => {
    setActiveChatId(chatId);
  }, []);

  const handleBack = useCallback(() => {
    setActiveChatId(null);
  }, []);

  // ---------- синхронизация refs и обработчиков ----------

  useEffect(() => {
    sessionRef.current = session;
  }, [session]);

  useEffect(() => {
    activeChatIdRef.current = activeChatId;
  }, [activeChatId]);

  useEffect(() => {
    // Лайтбокс привязан к blob-url сообщения из ленты: при смене чата закрываем.
    setLightbox(null);
  }, [activeChatId]);

  useEffect(() => {
    meRef.current = me;
  }, [me]);

  useEffect(() => {
    chatsRef.current = chats;
  }, [chats]);

  useEffect(() => {
    messagesRef.current = messages;
  }, [messages]);

  useEffect(() => {
    onUnauthorizedRef.current = handleSessionExpired;
  }, [handleSessionExpired]);

  useEffect(() => {
    wsEventRef.current = handleWsEvent;
  });

  // ---------- загрузка профиля и чатов после входа ----------

  useEffect(() => {
    if (!session) {
      return;
    }
    let cancelled = false;
    setChatsLoading(true);
    void (async () => {
      try {
        const [meData, chatList] = await Promise.all([api.me(), api.listChats()]);
        if (cancelled) {
          return;
        }
        setMe(meData);
        setChats(sortChats(chatList));
      } catch (error) {
        if (cancelled) {
          return;
        }
        if (!(error instanceof ApiError && error.status === 401)) {
          toast.error(getErrorMessage(error));
        }
      } finally {
        if (!cancelled) {
          setChatsLoading(false);
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [session, api, toast]);

  // ---------- WebSocket-соединение ----------

  useEffect(() => {
    if (!session) {
      setWsStatus('offline');
      return;
    }
    const client = new WsClient({
      url: getWebSocketUrl(),
      token: session.token,
      handlers: {
        onStatus: setWsStatus,
        onEvent: (event) => wsEventRef.current(event),
        onAuthFailed: () => onUnauthorizedRef.current(),
      },
    });
    wsRef.current = client;
    client.connect();
    return () => {
      client.close();
      if (wsRef.current === client) {
        wsRef.current = null;
      }
      setWsStatus('offline');
    };
  }, [session]);

  useEffect(() => {
    if (activeChatId !== null && wsStatus === 'connected') {
      wsRef.current?.subscribe(activeChatId);
    }
  }, [activeChatId, wsStatus]);

  useEffect(() => {
    if (wsStatus !== 'connected') {
      if (typingStopTimerRef.current !== null) {
        window.clearTimeout(typingStopTimerRef.current);
        typingStopTimerRef.current = null;
      }
      typingSentRef.current = false;
      typingChatIdRef.current = null;
    }
  }, [wsStatus]);

  // ---------- глобальные шорткаты ----------

  useEffect(() => {
    function handleKeyDown(event: KeyboardEvent) {
      if (modal !== null || lightbox !== null) {
        return;
      }
      const target = event.target as HTMLElement | null;
      const isTyping =
        target !== null &&
        (target.tagName === 'INPUT' ||
          target.tagName === 'TEXTAREA' ||
          target.isContentEditable);
      const key = event.key.toLowerCase();

      // Ctrl/Cmd+K — фокус на поиск чатов (как в Telegram/Notion).
      if ((event.ctrlKey || event.metaKey) && key === 'k') {
        event.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
        return;
      }
      // «/» вне поля ввода — тоже фокус поиска.
      if (key === '/' && !isTyping && !event.ctrlKey && !event.metaKey && !event.altKey) {
        event.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      }
    }
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [modal, lightbox]);

  // ---------- история активного чата ----------

  useEffect(() => {
    if (!session) {
      return;
    }
    if (activeChatId === null) {
      stopTyping();
      clearTyping();
      setMessages([]);
      setDetails(null);
      setUnreadDividerMessageId(null);
      setHistoryLoading(false);
      setHasMore(false);
      setLoadingMore(false);
      loadingMoreRef.current = false;
      return;
    }

    let cancelled = false;
    stopTyping();
    clearTyping();
    setMessages([]);
    setDetails(null);
    setUnreadDividerMessageId(null);
    setHistoryLoading(true);
    setHasMore(false);
    setLoadingMore(false);
    loadingMoreRef.current = false;
    // Счётчик непрочитанных запоминаем до markChatRead — она его обнуляет,
    // а разделитель «Непрочитанные сообщения» строится по этому числу.
    const unreadAtOpen =
      chatsRef.current.find((chat) => chat.id === activeChatId)?.unread_count ??
      0;
    markChatRead(activeChatId);

    void (async () => {
      try {
        const [history, chatDetails] = await Promise.all([
          api.history(activeChatId, PAGE_SIZE, 0),
          api.chatDetails(activeChatId),
        ]);
        if (cancelled) {
          return;
        }
        const merged = mergeHistory(history, messagesRef.current);
        setMessages((current) => mergeHistory(history, current));
        setHasMore(history.length >= PAGE_SIZE);
        setDetails(chatDetails);
        if (unreadAtOpen > 0 && merged.length > 0) {
          const skip = Math.min(unreadAtOpen, merged.length);
          setUnreadDividerMessageId(merged[merged.length - skip].id);
        } else {
          setUnreadDividerMessageId(null);
        }
      } catch (error) {
        if (cancelled) {
          return;
        }
        if (!(error instanceof ApiError && error.status === 401)) {
          toast.error(getErrorMessage(error));
        }
      } finally {
        if (!cancelled) {
          setHistoryLoading(false);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [session, activeChatId, api, toast, markChatRead, stopTyping, clearTyping]);

  // ---------- title и таймеры ----------

  const unreadTotal = useMemo(
    () =>
      chats.reduce(
        (sum, chat) => sum + (chat.unread_count > 0 ? chat.unread_count : 0),
        0,
      ),
    [chats],
  );

  useEffect(() => {
    document.title = unreadTotal > 0 ? `(${unreadTotal}) KoSta` : 'KoSta';
  }, [unreadTotal]);

  useEffect(() => {
    return () => {
      typingTimersRef.current.forEach((timer) => window.clearTimeout(timer));
      typingTimersRef.current.clear();
      if (typingStopTimerRef.current !== null) {
        window.clearTimeout(typingStopTimerRef.current);
      }
      if (readTimerRef.current !== null) {
        window.clearTimeout(readTimerRef.current);
      }
    };
  }, []);

  const activeChat = useMemo(
    () => chats.find((chat) => chat.id === activeChatId) ?? null,
    [chats, activeChatId],
  );

  const typingLabel = useMemo(
    () =>
      buildTypingLabel(
        typingUsers,
        details?.participants ?? [],
        activeChat?.is_group === true,
      ),
    [typingUsers, details, activeChat],
  );

  if (!session) {
    return (
      <>
        <AuthScreen
          loading={authLoading}
          error={authError}
          onSubmit={handleAuth}
        />
        <ToastViewport toasts={toasts} onDismiss={remove} />
      </>
    );
  }

  return (
    <>
      <div className="app-shell flex h-[100dvh] w-full overflow-hidden">
        <Sidebar
          me={me}
          fallbackPhone={session.phone}
          chats={chats}
          loading={chatsLoading}
          activeChatId={activeChatId}
          wsStatus={wsStatus}
          theme={theme}
          searchInputRef={searchInputRef}
          onToggleTheme={toggleTheme}
          onSelectChat={handleSelectChat}
          onNewChat={() => setModal('new-chat')}
          onNewGroup={() => setModal('new-group')}
          onOpenProfile={() => setModal('profile')}
          className={activeChatId !== null ? 'hidden md:flex' : 'flex'}
        />

        <ChatPanel
          api={api}
          chat={activeChat}
          details={details}
          messages={messages}
          historyLoading={historyLoading}
          loadingMore={loadingMore}
          hasMore={hasMore}
          typingLabel={typingLabel}
          wsStatus={wsStatus}
          unreadDividerMessageId={unreadDividerMessageId}
          onBack={handleBack}
          onOpenInfo={() => {
            if (activeChatId !== null) {
              setModal('info');
            }
          }}
          onSend={handleSend}
          onUpload={handleUpload}
          onLoadMore={handleLoadMore}
          onTypingInput={handleTypingInput}
          onOpenImage={handleOpenImage}
          onDownload={handleMediaDownload}
          onCopyMessage={handleCopyMessage}
          initialDraft={activeChatId !== null ? drafts.get(activeChatId) : ''}
          onDraftChange={(chatId, text) => drafts.set(chatId, text)}
          className={activeChatId !== null ? 'flex' : 'hidden md:flex'}
        />
      </div>

      <ToastViewport toasts={toasts} onDismiss={remove} />

      {lightbox && (
        <Lightbox
          src={lightbox.url}
          title={lightbox.message.media_name?.trim() || 'Изображение'}
          caption={lightbox.message.text.trim() || undefined}
          onClose={closeLightbox}
          onDownload={() => handleMediaDownload(lightbox.message)}
        />
      )}

      {modal === 'new-chat' && (
        <NewChatModal
          api={api}
          toast={toast}
          onClose={() => setModal(null)}
          onCreated={handleChatCreated}
        />
      )}

      {modal === 'new-group' && (
        <NewGroupModal
          api={api}
          toast={toast}
          onClose={() => setModal(null)}
          onCreated={handleChatCreated}
        />
      )}

      {modal === 'info' && activeChatId !== null && (
        <ChatInfoModal
          api={api}
          toast={toast}
          chatId={activeChatId}
          myUserId={me?.user_id ?? 0}
          initialDetails={details}
          onClose={() => setModal(null)}
          onDetailsChange={handleDetailsUpdated}
          onChatsChanged={() => void refreshChats({ silent: true })}
          onLeftChat={handleLeftChat}
        />
      )}

      {modal === 'profile' && (
        <ProfileModal
          me={me}
          fallbackPhone={session.phone}
          theme={theme}
          onThemeChange={setTheme}
          wsStatus={wsStatus}
          onLogout={handleLogout}
          onClose={() => setModal(null)}
        />
      )}
    </>
  );
}
