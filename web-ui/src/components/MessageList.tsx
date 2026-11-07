import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { ChevronUp, MessageSquare } from 'lucide-react';
import type { Participant, UiMessage } from '../types';
import { MessageBubble } from './MessageBubble';
import { Spinner } from './ui/Spinner';
import { dayKey, formatDayLabel } from '../utils/format';
import { cn } from '../utils/classNames';

const GROUP_WINDOW_SECONDS = 5 * 60;
const NEAR_BOTTOM_PX = 120;

interface MessageListProps {
  chatId: number;
  messages: UiMessage[];
  loading: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  typingLabel: string | null;
  isGroup: boolean;
  participants: Participant[];
  onLoadMore: () => Promise<boolean>;
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
  onLoadMore,
}: MessageListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const nearBottomRef = useRef(true);
  const previousChatIdRef = useRef<number | null>(null);
  const previousLastIdRef = useRef<number | null>(null);
  const restoreRef = useRef<{ height: number; top: number; firstId: number } | null>(
    null,
  );
  const [restoring, setRestoring] = useState(false);

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
    }
  }, [messages, loading, chatId, restoring]);

  function handleScroll() {
    const element = scrollRef.current;
    if (!element) {
      return;
    }
    const distance = element.scrollHeight - element.scrollTop - element.clientHeight;
    nearBottomRef.current = distance < NEAR_BOTTOM_PX;
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
          <span className="rounded-full border border-line bg-panel-soft px-3 py-1 text-[11px] font-medium text-muted">
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
        ?.phone ?? null;

    items.push(
      <MessageBubble
        key={message.id}
        message={message}
        isOwn={message.is_outgoing}
        inGroup={isGroup}
        showSender={!groupedWithPrevious}
        senderPhone={senderPhone}
        tail={!groupedWithNext}
      />,
    );

    previousMessage = message;
  }

  const showEmpty = !loading && messages.length === 0;

  return (
    <div
      ref={scrollRef}
      onScroll={handleScroll}
      className="kosta-scroll min-h-0 flex-1 overflow-y-auto px-3 py-4 md:px-6"
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
                <span className="mb-3 flex h-14 w-14 items-center justify-center rounded-2xl bg-panel-soft text-muted">
                  <MessageSquare className="h-6 w-6" aria-hidden="true" />
                </span>
                <p className="text-sm font-medium text-ink">Сообщений пока нет</p>
                <p className="mt-1 text-xs text-muted">Напишите первым — начните диалог</p>
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
  );
}
