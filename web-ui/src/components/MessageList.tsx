import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { ChevronDown, ChevronUp, MessageSquare } from 'lucide-react';
import type { Api } from '../api';
import type { Participant, UiMessage } from '../types';
import { MessageBubble } from './MessageBubble';
import { Spinner } from './ui/Spinner';
import { dayKey, formatDayLabel, userLabel } from '../utils/format';
import { cn } from '../utils/classNames';

const GROUP_WINDOW_SECONDS = 5 * 60;
const NEAR_BOTTOM_PX = 120;

// Устойчивые цвета имён участников в группах (привязаны к sender_id).
const SENDER_COLORS = [
  '#e08e52',
  '#c58f3c',
  '#5f9e77',
  '#5e8fb4',
  '#c47d9a',
  '#9a7bbf',
];

interface MessageListProps {
  chatId: number;
  messages: UiMessage[];
  loading: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  typingLabel: string | null;
  isGroup: boolean;
  participants: Participant[];
  api: Api;
  /** id текущего пользователя — чтобы подписать цитату «Вы». */
  myUserId: number;
  /** id первого сообщения из непрочитанного блока: над ним рисуем разделитель. */
  unreadDividerMessageId: number | null;
  onLoadMore: () => Promise<boolean>;
  onOpenImage: (message: UiMessage, url: string) => void;
  onDownload: (message: UiMessage) => Promise<void>;
  onCopyMessage: (message: UiMessage) => void;
  onReplyMessage: (message: UiMessage) => void;
  onEditMessage: (message: UiMessage) => void;
  onDeleteMessage: (message: UiMessage) => void;
  onToggleReaction: (message: UiMessage, emoji: string) => void;
  /** id закреплённого сообщения диалога (или null). */
  pinnedMessageId: number | null;
  /** Может ли пользователь закреплять сообщения. */
  canPin: boolean;
  onTogglePin: (message: UiMessage) => void;
  /** Сообщение, к которому нужно прокрутить ленту (из глобального поиска). */
  scrollToMessageId: number | null;
  /** Прокрутка выполнена — приложение может сбросить цель. */
  onScrollToMessageDone?: () => void;
}

function MessagesSkeleton() {
  return (
    <div className="space-y-3 py-4" aria-hidden="true">
      {Array.from({ length: 6 }, (_, index) => {
        const own = index % 2 === 1;
        return (
          <div key={index} className={cn('flex', own ? 'justify-end' : 'justify-start')}>
            <div
              className={cn(
                'h-12 animate-pulse rounded-2xl bg-panel-strong',
                own ? 'w-2/5' : 'w-1/2',
              )}
            />
          </div>
        );
      })}
    </div>
  );
}

export function MessageList({
  chatId,
  messages,
  loading,
  loadingMore,
  hasMore,
  typingLabel,
  isGroup,
  participants,
  api,
  myUserId,
  unreadDividerMessageId,
  onLoadMore,
  onOpenImage,
  onDownload,
  onCopyMessage,
  onReplyMessage,
  onEditMessage,
  onDeleteMessage,
  onToggleReaction,
  pinnedMessageId,
  canPin,
  onTogglePin,
  scrollToMessageId,
  onScrollToMessageDone,
}: MessageListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const nearBottomRef = useRef(true);
  const previousChatIdRef = useRef<number | null>(null);
  const previousLastIdRef = useRef<number | null>(null);
  const restoreRef = useRef<{ height: number; top: number; firstId: number } | null>(
    null,
  );
  const newBelowRef = useRef(0);
  const [restoring, setRestoring] = useState(false);
  const [showJump, setShowJump] = useState(false);
  const [newBelow, setNewBelow] = useState(0);
  // Указываем «есть более ранние сообщения» мягкой тенью сверху, когда ушли вниз.
  const [atTop, setAtTop] = useState(false);

  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element) {
      return;
    }

    const firstId = messages.length > 0 ? messages[0].id : 0;
    const lastId = messages.length > 0 ? messages[messages.length - 1].id : null;
    const chatChanged = previousChatIdRef.current !== chatId;
    const lastChanged = previousLastIdRef.current !== lastId;

    previousChatIdRef.current = chatId;
    previousLastIdRef.current = lastId;

    if (restoreRef.current && restoreRef.current.firstId !== firstId) {
      const { height, top } = restoreRef.current;
      restoreRef.current = null;
      element.scrollTop = element.scrollHeight - height + top;
      if (restoring) {
        setRestoring(false);
      }
      return;
    }

    const lastMessage = messages.length > 0 ? messages[messages.length - 1] : null;
    const shouldStick =
      chatChanged ||
      (lastChanged &&
        (nearBottomRef.current || lastMessage?.is_outgoing === true));

    if (shouldStick) {
      element.scrollTop = element.scrollHeight;
      nearBottomRef.current = true;
      newBelowRef.current = 0;
      setNewBelow(0);
      setShowJump(false);
    } else if (lastChanged && !restoreRef.current && lastMessage) {
      // Новое сообщение, пока пользователь прокрутил ленту вверх:
      // копим счётчик на кнопке «вниз».
      newBelowRef.current += 1;
      setNewBelow(newBelowRef.current);
      setShowJump(true);
    }
  }, [messages, loading, chatId, restoring]);

  // Прокрутка к сообщению из глобального поиска. Ждём окончания загрузки,
  // иначе цель ещё не отрисована. Если сообщение не попало в загруженную
  // страницу, цель остаётся — повторная попытка будет при догрузке истории.
  useEffect(() => {
    if (scrollToMessageId === null || loading) {
      return;
    }
    const element = scrollRef.current;
    if (!element) {
      return;
    }
    const target = element.querySelector<HTMLElement>(
      `[data-message-id="${scrollToMessageId}"]`,
    );
    if (!target) {
      return;
    }
    target.scrollIntoView({ block: 'center', behavior: 'smooth' });
    nearBottomRef.current = false;
    setShowJump(true);
    onScrollToMessageDone?.();
  }, [scrollToMessageId, messages.length, loading, onScrollToMessageDone]);

  function handleScroll() {
    const element = scrollRef.current;
    if (!element) {
      return;
    }
    const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
    const nearBottom = distance < NEAR_BOTTOM_PX;
    const top = element.scrollTop <= 2;
    nearBottomRef.current = nearBottom;
    setShowJump(!nearBottom);
    setAtTop(top);
    if (nearBottom && newBelowRef.current > 0) {
      newBelowRef.current = 0;
      setNewBelow(0);
    }
  }

  function handleJumpToBottom() {
    const element = scrollRef.current;
    if (!element) {
      return;
    }
    element.scrollTo({ top: element.scrollHeight, behavior: 'smooth' });
    nearBottomRef.current = true;
    setAtTop(false);
    newBelowRef.current = 0;
    setNewBelow(0);
    setShowJump(false);
  }

  async function handleLoadMoreClick() {
    const element = scrollRef.current;
    if (!element || loadingMore) {
      return;
    }
    if (messages.length > 0) {
      restoreRef.current = {
        height: element.scrollHeight,
        top: element.scrollTop,
        firstId: messages[0].id,
      };
      setRestoring(true);
    }
    const added = await onLoadMore();
    if (!added || messages.length === 0) {
      restoreRef.current = null;
      setRestoring(false);
    }
  }

  const items: ReactNode[] = [];
  let previousMessage: UiMessage | null = null;
  let currentDay = '';

  for (let index = 0; index < messages.length; index += 1) {
    const message = messages[index];
    const messageDay = dayKey(message.timestamp);

    if (messageDay !== currentDay) {
      currentDay = messageDay;
      previousMessage = null;
      items.push(
        <div key={`day-${messageDay}`} className="my-3 flex justify-center">
          <span className="rounded-full border border-line-soft bg-panel-soft/80 px-2.5 py-1 text-[11px] text-muted">
            {formatDayLabel(message.timestamp)}
          </span>
        </div>,
      );
    }

    const previous = previousMessage;
    const next = messages[index + 1];
    const groupedWithPrevious =
      previous !== null &&
      previous.sender_id === message.sender_id &&
      previous.is_outgoing === message.is_outgoing &&
      message.timestamp - previous.timestamp < GROUP_WINDOW_SECONDS;
    const groupedWithNext =
      next !== undefined &&
      next.sender_id === message.sender_id &&
      next.is_outgoing === message.is_outgoing &&
      next.timestamp - message.timestamp < GROUP_WINDOW_SECONDS;

    const senderPhone =
      participants.find((participant) => participant.user_id === message.sender_id)
        ?.display_name?.trim() ||
      participants.find((participant) => participant.user_id === message.sender_id)
        ?.phone ||
      null;

    // Подпись цитаты: «Вы» для своих, телефон участника для чужих,
    // отдельная строка для удалённого сообщения.
    let replyAuthor: string | null = null;
    if (message.reply_to_id != null) {
      if (message.reply_to_sender_id == null) {
        replyAuthor = 'Сообщение удалено';
      } else if (message.reply_to_sender_id === myUserId) {
        replyAuthor = 'Вы';
      } else {
        const author = participants.find(
          (participant) => participant.user_id === message.reply_to_sender_id,
        );
        replyAuthor = author ? userLabel(author) : 'Участник';
      }
    }

    if (
      unreadDividerMessageId !== null &&
      message.id === unreadDividerMessageId
    ) {
      items.push(
        <div
          key={`unread-${message.id}`}
          role="separator"
          aria-label="Непрочитанные сообщения"
          className="my-3 flex justify-center"
        >
          <span className="rounded-full bg-accent-soft px-3 py-1 text-[11px] font-semibold text-accent-strong ring-1 ring-accent/25">
            Непрочитанные сообщения
          </span>
        </div>,
      );
    }

    items.push(
      <MessageBubble
        key={message.id}
        message={message}
        isOwn={message.is_outgoing}
        inGroup={isGroup}
        showSender={!groupedWithPrevious}
        senderPhone={senderPhone}
        senderColor={
          isGroup && !message.is_outgoing
            ? SENDER_COLORS[message.sender_id % SENDER_COLORS.length]
            : undefined
        }
        replyAuthor={replyAuthor}
        tail={!groupedWithNext}
        api={api}
        onOpenImage={onOpenImage}
        onDownload={onDownload}
        onCopyMessage={onCopyMessage}
        onReplyMessage={onReplyMessage}
        onEditMessage={onEditMessage}
        onDeleteMessage={onDeleteMessage}
        onToggleReaction={onToggleReaction}
        isPinned={pinnedMessageId === message.id}
        canPin={canPin}
        onTogglePin={onTogglePin}
      />,
    );

    previousMessage = message;
  }

  const showEmpty = !loading && messages.length === 0;

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="chat-wallpaper kosta-scroll min-h-0 flex-1 overflow-y-auto px-3 py-4 md:px-6"
        aria-live="polite"
        aria-label="История сообщений"
      >
        <div className="mx-auto flex w-full max-w-3xl flex-col gap-1">
        {loading ? (
          <MessagesSkeleton />
        ) : (
          <>
            {hasMore && (
              <div className="mb-2 flex justify-center">
                <button
                  type="button"
                  onClick={() => void handleLoadMoreClick()}
                  disabled={loadingMore}
                  className="flex items-center gap-1.5 rounded-full border border-line bg-panel px-4 py-1.5 text-xs font-medium text-muted transition-colors hover:bg-panel-soft hover:text-ink disabled:opacity-60"
                >
                  {loadingMore ? (
                    <Spinner className="h-3.5 w-3.5" label="Загрузка сообщений" />
                  ) : (
                    <ChevronUp className="h-3.5 w-3.5" aria-hidden="true" />
                  )}
                  Загрузить ещё
                </button>
              </div>
            )}

            {showEmpty ? (
              <div className="flex flex-col items-center justify-center px-6 py-16 text-center">
                <span className="mb-3 flex h-14 w-14 items-center justify-center rounded-2xl bg-accent-soft text-accent-strong">
                  <MessageSquare className="h-6 w-6" aria-hidden="true" />
                </span>
                <p className="text-sm font-medium text-ink">Сообщений пока нет</p>
                <p className="mt-1 max-w-xs text-xs leading-relaxed text-muted">
                  Напишите первым — начните диалог. Или прикрепите файл через
                  скрепку рядом с полем ввода.
                </p>
              </div>
            ) : (
              items
            )}

            {typingLabel && (
              <div className="animate-fade-in-up mt-1 flex justify-start">
                <div className="bubble-in flex items-center gap-2 rounded-2xl rounded-bl-md px-3.5 py-2.5">
                  <span className="flex items-center gap-1" aria-hidden="true">
                    <span className="typing-dot h-1.5 w-1.5 rounded-full bg-muted" />
                    <span
                      className="typing-dot h-1.5 w-1.5 rounded-full bg-muted"
                      style={{ animationDelay: '0.15s' }}
                    />
                    <span
                      className="typing-dot h-1.5 w-1.5 rounded-full bg-muted"
                      style={{ animationDelay: '0.3s' }}
                    />
                  </span>
                  <span className="text-[11px] text-muted">{typingLabel}</span>
                </div>
              </div>
            )}
          </>
        )}
        </div>
      </div>

      {/* Мягкая тень сверху — подсказка, что выше есть история. */}
      <div
        aria-hidden="true"
        className={cn(
          'pointer-events-none absolute inset-x-0 top-0 z-[5] h-5 bg-gradient-to-b from-black/20 to-transparent transition-opacity duration-200',
          atTop ? 'opacity-0' : 'opacity-100',
        )}
      />

      {showJump && (
        <button
          type="button"
          onClick={handleJumpToBottom}
          aria-label="К последнему сообщению"
          title="К последнему сообщению"
          className="animate-fade-in-up absolute bottom-4 right-4 z-10 flex h-11 w-11 items-center justify-center rounded-full border border-line bg-panel-strong text-ink shadow-md transition-colors hover:bg-panel-strong hover:text-accent-strong"
        >
          <ChevronDown className="h-5 w-5" aria-hidden="true" />
          {newBelow > 0 && (
            <span
              className="absolute -right-0.5 -top-0.5 grid h-5 min-w-5 place-items-center rounded-full bg-accent px-1 text-[10px] font-semibold leading-none text-white"
              role="status"
              aria-label={`Новых сообщений: ${newBelow}`}
            >
              {newBelow > 99 ? '99+' : newBelow}
            </span>
          )}
        </button>
      )}
    </div>
  );
}
