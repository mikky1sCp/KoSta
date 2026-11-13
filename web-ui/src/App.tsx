import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { ApiError, createApi, type Api } from './api';
import { WsClient, getWebSocketUrl, type WsEvent } from './ws';
import type {
  ChatDetails,
  ChatItem,
  Me,
  Message,
  UiMessage,
  UserSearchItem,
  WsStatus,
} from './types';
import { getErrorMessage } from './utils/errors';
import { buildTypingLabel } from './utils/format';
import { downloadMedia, messagePreview } from './utils/media';
import { useTheme } from './hooks/useTheme';
import { useToasts } from './hooks/useToasts';
import { useDrafts } from './hooks/useDrafts';
import { useNotifications } from './hooks/useNotifications';
import { clearStoredSession, loadStoredSession, saveStoredSession } from './utils/session';
import { AuthScreen } from './components/AuthScreen';
import { Sidebar } from './components/Sidebar';
import { ChatPanel } from './components/ChatPanel';
import { ToastViewport } from './components/ui/ToastViewport';
import { Lightbox } from './components/ui/Lightbox';
import { NewChatModal } from './components/modals/NewChatModal';
import { NewGroupModal } from './components/modals/NewGroupModal';
import { ChatInfoModal } from './components/modals/ChatInfoModal';
import { ProfileModal } from './components/modals/ProfileModal';
import { SearchPalette } from './components/SearchPalette';

const PAGE_SIZE = 50;
const TYPING_TIMEOUT_MS = 3_000;
const TYPING_THROTTLE_MS = 2_000;
const TYPING_IDLE_MS = 2_000;
const READ_DEBOUNCE_MS = 250;

interface Session {
  token: string;
  userId: number;
  phone: string;
  /** Сохранять ли сессию в localStorage («Запомнить меня»). */
  persist: boolean;
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
  const notifications = useNotifications();

  const [session, setSession] = useState<Session | null>(() => {
    const stored = loadStoredSession();
    return stored ? { ...stored, persist: true } : null;
  });
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
  /** Палитра глобального поиска (Ctrl+K). */
  const [paletteOpen, setPaletteOpen] = useState(false);
  /** Сообщение, к которому нужно прокрутить ленту после перехода из поиска. */
  const [scrollToMessageId, setScrollToMessageId] = useState<number | null>(null);
  const [lightbox, setLightbox] = useState<LightboxState | null>(null);
  const [unreadDividerMessageId, setUnreadDividerMessageId] = useState<
    number | null
  >(null);
  // Ответ на сообщение и режим правки — взаимоисключающие режимы композера.
  const [replyTo, setReplyTo] = useState<UiMessage | null>(null);
  const [editing, setEditing] = useState<UiMessage | null>(null);
  /** Закреплённое сообщение активного диалога. */
  const [pinnedMessage, setPinnedMessage] = useState<UiMessage | null>(null);

  const sessionRef = useRef<Session | null>(null);
  const activeChatIdRef = useRef<number | null>(null);
  const meRef = useRef<Me | null>(null);
  const chatsRef = useRef<ChatItem[]>([]);
  const messagesRef = useRef<UiMessage[]>([]);
  const wsRef = useRef<WsClient | null>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const replyToRef = useRef<UiMessage | null>(null);
  const pinnedMessageRef = useRef<UiMessage | null>(null);
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
    clearStoredSession();
    setSession(null);
    wsRef.current?.close();
    resetSessionState();
    toast.error('Сессия истекла. Войдите снова.');
  }, [resetSessionState, toast]);

  const handleAuth = useCallback(
    async (phone: string, password: string, register: boolean, remember: boolean) => {
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
          persist: remember,
        };
        if (remember) {
          saveStoredSession({
            token: nextSession.token,
            userId: nextSession.userId,
            phone: nextSession.phone,
          });
        } else {
          clearStoredSession();
        }
        sessionRef.current = nextSession;
        setSession(nextSession);
        setMe({
          user_id: response.user_id,
          phone,
          display_name: null,
          username: null,
          has_avatar: false,
          is_online: true,
        });
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
    clearStoredSession();
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

      const replyTarget = replyToRef.current;
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
        reply_to_id: replyTarget?.id ?? null,
        reply_to_text: replyTarget ? messagePreview(replyTarget) : null,
        reply_to_sender_id: replyTarget?.sender_id ?? null,
        reply_to_is_media: replyTarget?.is_media ?? false,
        edited_at: null,
        reactions: [],
        pending: true,
      };

      if (!client.sendMessage(chatId, text, replyTarget?.id ?? null)) {
        toast.error('Не удалось отправить сообщение');
        return false;
      }
      stopTyping();
      setReplyTo(null);
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

  // ---------- ответы, правка, удаление ----------

  const handleReplyMessage = useCallback((message: UiMessage) => {
    setEditing(null);
    setReplyTo(message);
  }, []);

  const handleEditMessage = useCallback((message: UiMessage) => {
    setReplyTo(null);
    setEditing(message);
  }, []);

  const handleCancelReply = useCallback(() => setReplyTo(null), []);
  const handleCancelEdit = useCallback(() => setEditing(null), []);

  const handleEditSubmit = useCallback(
    (text: string) => {
      const target = editing;
      if (!target) {
        return;
      }
      const trimmed = text.trim();
      if (!trimmed) {
        return;
      }
      const now = Math.floor(Date.now() / 1000);
      // Оптимистично обновляем ленту; серверное эхо `message_edited`
      // затем перезапишет те же поля (идемпотентно).
      setMessages((current) =>
        current.map((message) =>
          message.id === target.id
            ? { ...message, text: trimmed, edited_at: now }
            : message,
        ),
      );
      setChats((current) =>
        current.map((chat) =>
          chat.id === activeChatIdRef.current && chat.last_message === target.text
            ? { ...chat, last_message: trimmed }
            : chat,
        ),
      );
      setEditing(null);
      void api.editMessage(target.id, trimmed).catch((error) => {
        if (!(error instanceof ApiError && error.status === 401)) {
          toast.error(getErrorMessage(error));
        }
        // Возвращаем исходный текст, если сервер отказал.
        setMessages((current) =>
          current.map((message) =>
            message.id === target.id
              ? { ...message, text: target.text, edited_at: target.edited_at }
              : message,
          ),
        );
      });
    },
    [api, editing, toast],
  );

  const handleDeleteMessage = useCallback(    (message: UiMessage) => {
      if (!window.confirm('Удалить сообщение?')) {
        return;
      }
      setMessages((current) => current.filter((item) => item.id !== message.id));
      setReplyTo((current) => (current?.id === message.id ? null : current));
      setEditing((current) => (current?.id === message.id ? null : current));
      void api.deleteMessage(message.id).catch((error) => {
        if (!(error instanceof ApiError && error.status === 401)) {
          toast.error(getErrorMessage(error));
        }
        void refreshChats({ silent: true });
      });
    },
    [api, refreshChats, toast],
  );

  const handleToggleReaction = useCallback(
    (message: UiMessage, emoji: string) => {
      if (message.id <= 0) {
        return;
      }
      // Оптимистично переключаем у себя, затем сверяемся с сервером.
      setMessages((current) =>
        current.map((item) => {
          if (item.id !== message.id) {
            return item;
          }
          const existing = item.reactions.find((reaction) => reaction.emoji === emoji);
          if (!existing) {
            return {
              ...item,
              reactions: [...item.reactions, { emoji, count: 1, me: true }],
            };
          }
          if (existing.me) {
            return {
              ...item,
              reactions: item.reactions
                .map((reaction) =>
                  reaction.emoji === emoji
                    ? { ...reaction, count: reaction.count - 1, me: false }
                    : reaction,
                )
                .filter((reaction) => reaction.count > 0),
            };
          }
          return {
            ...item,
            reactions: item.reactions.map((reaction) =>
              reaction.emoji === emoji
                ? { ...reaction, count: reaction.count + 1, me: true }
                : reaction,
            ),
          };
        }),
      );

      void api
        .toggleReaction(message.id, emoji)
        .then((response) => {
          setMessages((current) =>
            current.map((item) =>
              item.id === message.id
                ? { ...item, reactions: response.reactions ?? [] }
                : item,
            ),
          );
        })
        .catch((error) => {
          if (!(error instanceof ApiError && error.status === 401)) {
            toast.error(getErrorMessage(error));
          }
          void refreshChats({ silent: true });
        });
    },
    [api, refreshChats, toast],
  );

  const handleTogglePin = useCallback(
    (message: UiMessage) => {
      const chatId = activeChatIdRef.current;
      if (chatId === null || message.id <= 0) {
        return;
      }
      const unpin = pinnedMessageRef.current?.id === message.id;
      void api
        .pinMessage(chatId, unpin ? null : message.id)
        .then((response) => {
          setPinnedMessage(
            response.pinned_message
              ? { ...response.pinned_message, pending: false }
              : null,
          );
        })
        .catch((error) => {
          if (!(error instanceof ApiError && error.status === 401)) {
            toast.error(getErrorMessage(error));
          }
        });
    },
    [api, toast],
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

        // Уведомление и звук — только для входящих: свои сообщения
        // пользователь и так видит.
        if (!isOutgoing) {
          notifications.ping();
          const chatTitle =
            chatsRef.current.find((chat) => chat.id === dialogId)?.title ??
            'KoSta';
          notifications.notify(chatTitle, messagePreview(uiMessage), () => {
            setActiveChatId(dialogId);
          });
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

      case 'message_edited': {
        const { dialog_id, message_id, text, edited_at } = event.payload;
        if (dialog_id !== activeChatIdRef.current) {
          // Лента не открыта: актуальный текст придёт с историей при открытии.
          break;
        }
        setMessages((current) =>
          current.map((message) =>
            message.id === message_id
              ? { ...message, text, edited_at }
              : message,
          ),
        );
        break;
      }

      case 'message_deleted': {
        const { dialog_id, message_id } = event.payload;
        setReplyTo((current) => (current?.id === message_id ? null : current));
        setEditing((current) => (current?.id === message_id ? null : current));
        if (dialog_id !== activeChatIdRef.current) {
          break;
        }
        setMessages((current) =>
          current.filter((message) => message.id !== message_id),
        );
        break;
      }

      case 'message_reactions': {
        const { dialog_id, message_id, reactions } = event.payload;
        if (dialog_id !== activeChatIdRef.current) {
          break;
        }
        setMessages((current) =>
          current.map((item) => {
            if (item.id !== message_id) {
              return item;
            }
            // Сервер присылает только счётчики: свой флаг «me» сохраняем.
            const previous = new Map(
              item.reactions.map((reaction) => [reaction.emoji, reaction.me]),
            );
            return {
              ...item,
              reactions: reactions.map((reaction) => ({
                emoji: reaction.emoji,
                count: reaction.count,
                me: previous.get(reaction.emoji) ?? false,
              })),
            };
          }),
        );
        break;
      }

      case 'dialog_pinned': {
        const { dialog_id, message } = event.payload;
        if (dialog_id !== activeChatIdRef.current) {
          break;
        }
        setPinnedMessage(message ? { ...message, pending: false } : null);
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
    setScrollToMessageId(null);
    setActiveChatId(chatId);
  }, []);

  const handleBack = useCallback(() => {
    setScrollToMessageId(null);
    setActiveChatId(null);
  }, []);

  /** Переход к чату из палитры поиска. */
  const handleOpenChat = useCallback((chatId: number) => {
    setPaletteOpen(false);
    setScrollToMessageId(null);
    setActiveChatId(chatId);
  }, []);

  /** Переход к конкретному сообщению из глобального поиска. */
  const handleOpenMessage = useCallback((chatId: number, messageId: number) => {
    setPaletteOpen(false);
    setActiveChatId(chatId);
    setScrollToMessageId(messageId);
  }, []);

  /** Начать личный чат с человеком, найденным через поиск людей. */
  const handleStartChat = useCallback(
    async (user: UserSearchItem) => {
      setPaletteOpen(false);
      if (!user.username) {
        toast.error('У пользователя нет username');
        return;
      }
      try {
        const next = await api.createPrivate('', user.username);
        handleChatCreated(next);
      } catch (error) {
        toast.error(getErrorMessage(error));
      }
    },
    [api, handleChatCreated, toast],
  );

  const handleScrollToMessageDone = useCallback(() => {
    setScrollToMessageId(null);
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
    // Ответ и правка относятся к сообщению конкретного чата.
    setReplyTo(null);
    setEditing(null);
    setPinnedMessage(null);
  }, [activeChatId]);

  useEffect(() => {
    meRef.current = me;
  }, [me]);

  useEffect(() => {
    replyToRef.current = replyTo;
  }, [replyTo]);

  useEffect(() => {
    pinnedMessageRef.current = pinnedMessage;
  }, [pinnedMessage]);

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
      if (modal !== null || lightbox !== null || paletteOpen) {
        return;
      }
      const target = event.target as HTMLElement | null;
      const isTyping =
        target !== null &&
        (target.tagName === 'INPUT' ||
          target.tagName === 'TEXTAREA' ||
          target.isContentEditable);
      const key = event.key.toLowerCase();

      // Ctrl/Cmd+K — палитра глобального поиска (как в Telegram/Notion).
      if ((event.ctrlKey || event.metaKey) && key === 'k') {
        event.preventDefault();
        setPaletteOpen(true);
        return;
      }
      // «/» вне поля ввода — фокус на поиск чатов в сайдбаре.
      if (key === '/' && !isTyping && !event.ctrlKey && !event.metaKey && !event.altKey) {
        event.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
      }
    }
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [modal, lightbox, paletteOpen]);

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
        setPinnedMessage(
          chatDetails.pinned_message
            ? { ...chatDetails.pinned_message, pending: false }
            : null,
        );
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

  // Представление ответа для композера: автор + короткий текст цитаты.
  const replyToView = useMemo(() => {
    if (!replyTo) {
      return null;
    }
    const author = replyTo.is_outgoing
      ? 'Вы'
      : details?.participants.find(
          (participant) => participant.user_id === replyTo.sender_id,
        )?.phone ?? 'Участник';
    return { author, text: messagePreview(replyTo) };
  }, [replyTo, details]);

  const editingView = useMemo(
    () => (editing ? { text: editing.text } : null),
    [editing],
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
          myUserId={me?.user_id ?? 0}
          replyTo={replyToView}
          editing={editingView}
          onCancelReply={handleCancelReply}
          onCancelEdit={handleCancelEdit}
          onEditSubmit={handleEditSubmit}
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
          onReplyMessage={handleReplyMessage}
          onEditMessage={handleEditMessage}
          onDeleteMessage={handleDeleteMessage}
          onToggleReaction={handleToggleReaction}
          pinnedMessage={pinnedMessage}
          canPin={activeChat?.is_group ? (details?.is_admin ?? false) : true}
          onTogglePin={handleTogglePin}
          scrollToMessageId={scrollToMessageId}
          onScrollToMessageDone={handleScrollToMessageDone}
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
          api={api}
          me={me}
          fallbackPhone={session.phone}
          theme={theme}
          onThemeChange={setTheme}
          wsStatus={wsStatus}
          notifications={notifications}
          sessionPersisted={session.persist}
          onTogglePersistence={(persist) => {
            setSession((current) => (current ? { ...current, persist } : current));
            if (persist) {
              saveStoredSession({
                token: session.token,
                userId: session.userId,
                phone: session.phone,
              });
            } else {
              clearStoredSession();
            }
          }}
          onProfileChange={(next) => {
            setMe(next);
            // Имя влияет на заголовки личных чатов — обновляем список.
            void refreshChats({ silent: true });
          }}
          onLogout={handleLogout}
          onClose={() => setModal(null)}
        />
      )}
      {paletteOpen && (
        <SearchPalette
          api={api}
          chats={chats}
          onClose={() => setPaletteOpen(false)}
          onOpenChat={handleOpenChat}
          onOpenMessage={handleOpenMessage}
          onStartChat={(user) => void handleStartChat(user)}
        />
      )}
    </>
  );
}
