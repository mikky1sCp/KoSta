import { useEffect, useRef, useState } from 'react';
import { Loader2, MessageCircle, Search, X } from 'lucide-react';
import type { Api } from '../api';
import type { ChatDetails, ChatItem, UiMessage, WsStatus } from '../types';
import { ChatHeader } from './ChatHeader';
import { Composer } from './Composer';
import { MessageList } from './MessageList';
import { cn } from '../utils/classNames';

interface ChatPanelProps {
  api: Api;
  chat: ChatItem | null;
  details: ChatDetails | null;
  messages: UiMessage[];
  historyLoading: boolean;
  loadingMore: boolean;
  hasMore: boolean;
  typingLabel: string | null;
  wsStatus: WsStatus;
  /** id первого сообщения непрочитанного блока — над ним рисуем разделитель. */
  unreadDividerMessageId: number | null;
  /** id текущего пользователя для подписи цитат. */
  myUserId: number;
  /** Сообщение, на которое сейчас отвечаем (или null). */
  replyTo: { author: string; text: string } | null;
  /** Сообщение, которое сейчас редактируем (или null). */
  editing: { text: string } | null;
  onCancelReply: () => void;
  onCancelEdit: () => void;
  onEditSubmit: (text: string) => void;
  onBack: () => void;
  onOpenInfo: () => void;
  onSend: (text: string) => boolean;
  onUpload: (
    file: File,
    caption: string,
    onProgress: (percent: number) => void,
  ) => Promise<boolean>;
  onLoadMore: () => Promise<boolean>;
  onTypingInput: (hasText: boolean) => void;
  onOpenImage: (message: UiMessage, url: string) => void;
  onDownload: (message: UiMessage) => Promise<void>;
  onCopyMessage: (message: UiMessage) => void;
  onReplyMessage: (message: UiMessage) => void;
  onEditMessage: (message: UiMessage) => void;
  onDeleteMessage: (message: UiMessage) => void;
  initialDraft: string;
  onDraftChange: (chatId: number, text: string) => void;
  className?: string;
}

function EmptyChat() {
  return (
    <div className="flex h-full flex-1 flex-col items-center justify-center px-6 text-center">
      <div className="relative mb-5">
        <span className="flex h-14 w-14 items-center justify-center rounded-2xl bg-accent text-white">
          <MessageCircle className="h-7 w-7" strokeWidth={2.2} aria-hidden="true" />
        </span>
        <span className="absolute -bottom-1 -right-1 flex h-7 w-7 items-center justify-center rounded-full border border-line bg-panel text-accent-strong">
          <Search className="h-3.5 w-3.5" aria-hidden="true" />
        </span>
      </div>

      <h2 className="text-lg font-semibold tracking-tight text-ink">
        Добро пожаловать в KoSta
      </h2>
      <p className="mt-2 max-w-sm text-sm leading-relaxed text-muted">
        Выберите чат слева или начните новый диалог. История и непрочитанные
        подтянутся автоматически.
      </p>
      <p className="mt-4 inline-flex items-center gap-2 rounded-full border border-line bg-panel px-3 py-1.5 text-[11px] text-muted">
        <kbd className="rounded-md border border-line bg-panel-soft px-1.5 py-0.5 font-sans text-[10px] font-medium text-ink">
          Ctrl+K
        </kbd>
        поиск по чатам
      </p>
    </div>
  );
}

/** Панель открытого чата. Вынесена отдельно, чтобы поиск имел своё состояние. */
function ChatView({
  api,
  chat,
  details,
  messages,
  historyLoading,
  loadingMore,
  hasMore,
  typingLabel,
  wsStatus,
  unreadDividerMessageId,
  myUserId,
  replyTo,
  editing,
  onCancelReply,
  onCancelEdit,
  onEditSubmit,
  onBack,
  onOpenInfo,
  onSend,
  onUpload,
  onLoadMore,
  onTypingInput,
  onOpenImage,
  onDownload,
  onCopyMessage,
  onReplyMessage,
  onEditMessage,
  onDeleteMessage,
  initialDraft,
  onDraftChange,
}: Omit<ChatPanelProps, 'className' | 'chat'> & { chat: ChatItem }) {
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<UiMessage[] | null>(null);
  const [searching, setSearching] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);

  // Смена чата закрывает поиск: результаты относятся к прошлому диалогу.
  useEffect(() => {
    setSearchOpen(false);
    setQuery('');
    setResults(null);
    setSearching(false);
  }, [chat.id]);

  useEffect(() => {
    if (searchOpen) {
      searchInputRef.current?.focus();
    }
  }, [searchOpen]);

  // Поиск с задержкой: не дёргаем сервер на каждую букву.
  useEffect(() => {
    if (!searchOpen) {
      return;
    }
    const needle = query.trim();
    if (needle.length === 0) {
      setResults(null);
      setSearching(false);
      return;
    }
    let cancelled = false;
    setSearching(true);
    const timer = window.setTimeout(() => {
      void api
        .searchMessages(chat.id, needle)
        .then((rows) => {
          if (cancelled) {
            return;
          }
          setResults(rows.map((message) => ({ ...message, pending: false })));
        })
        .catch(() => {
          if (!cancelled) {
            setResults([]);
          }
        })
        .finally(() => {
          if (!cancelled) {
            setSearching(false);
          }
        });
    }, 300);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [api, chat.id, searchOpen, query]);

  const showingSearch = searchOpen && query.trim().length > 0;

  return (
    <>
      <ChatHeader
        chat={chat}
        details={details}
        typingLabel={typingLabel}
        searchOpen={searchOpen}
        onToggleSearch={() => setSearchOpen((open) => !open)}
        onBack={onBack}
        onOpenInfo={onOpenInfo}
      />

      {searchOpen && (
        <div className="border-b border-line bg-panel px-3 py-2 md:px-5">
          <div className="mx-auto flex w-full max-w-3xl items-center gap-2 rounded-xl border border-line bg-panel-soft px-3">
            <Search className="h-4 w-4 shrink-0 text-muted" aria-hidden="true" />
            <input
              ref={searchInputRef}
              type="search"
              value={query}
              onChange={(event) => setQuery(event.target.value)}
              placeholder="Поиск по сообщениям"
              aria-label="Поиск по сообщениям"
              className="h-9 w-full bg-transparent text-sm text-ink outline-none placeholder:text-muted"
            />
            {query.length > 0 && (
              <button
                type="button"
                onClick={() => setQuery('')}
                aria-label="Очистить поиск"
                className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-muted transition-colors hover:bg-panel-strong hover:text-ink"
              >
                <X className="h-3.5 w-3.5" aria-hidden="true" />
              </button>
            )}
          </div>
        </div>
      )}

      {wsStatus !== 'connected' && (
        <div className="flex items-center justify-center gap-2 border-b border-line bg-warn/5 px-4 py-1.5 text-[11px] text-warn">
          <Loader2 className="h-3 w-3 animate-spin" aria-hidden="true" />
          Восстанавливаем соединение…
        </div>
      )}

      {showingSearch ? (
        <div className="flex min-h-0 flex-1 flex-col">
          <div className="flex items-center justify-between px-4 py-2 text-[11px] text-muted">
            <span>
              {searching
                ? 'Ищем…'
                : results === null
                  ? ''
                  : `Найдено: ${results.length}`}
            </span>
            <button
              type="button"
              onClick={() => setSearchOpen(false)}
              className="rounded-md px-2 py-0.5 transition-colors hover:bg-panel-soft hover:text-ink"
            >
              Закрыть
            </button>
          </div>
          {results !== null && results.length === 0 && !searching ? (
            <div className="flex flex-1 flex-col items-center justify-center px-6 text-center">
              <span className="mb-3 flex h-12 w-12 items-center justify-center rounded-2xl bg-panel-soft text-muted">
                <Search className="h-5 w-5" aria-hidden="true" />
              </span>
              <p className="text-sm font-medium text-ink">Ничего не найдено</p>
              <p className="mt-1 text-xs text-muted">Попробуйте другой запрос</p>
            </div>
          ) : (
            <MessageList
              chatId={chat.id}
              messages={results ?? []}
              loading={searching || results === null}
              loadingMore={false}
              hasMore={false}
              typingLabel={null}
              isGroup={chat.is_group}
              participants={details?.participants ?? []}
              api={api}
              myUserId={myUserId}
              unreadDividerMessageId={null}
              onLoadMore={async () => false}
              onOpenImage={onOpenImage}
              onDownload={onDownload}
              onCopyMessage={onCopyMessage}
              onReplyMessage={onReplyMessage}
              onEditMessage={onEditMessage}
              onDeleteMessage={onDeleteMessage}
            />
          )}
        </div>
      ) : (
        <MessageList
          chatId={chat.id}
          messages={messages}
          loading={historyLoading}
          loadingMore={loadingMore}
          hasMore={hasMore}
          typingLabel={typingLabel}
          isGroup={chat.is_group}
          participants={details?.participants ?? []}
          api={api}
          myUserId={myUserId}
          unreadDividerMessageId={unreadDividerMessageId}
          onLoadMore={onLoadMore}
          onOpenImage={onOpenImage}
          onDownload={onDownload}
          onCopyMessage={onCopyMessage}
          onReplyMessage={onReplyMessage}
          onEditMessage={onEditMessage}
          onDeleteMessage={onDeleteMessage}
        />
      )}

      <Composer
        key={chat.id}
        chatId={chat.id}
        connected={wsStatus === 'connected'}
        initialDraft={initialDraft}
        replyTo={replyTo}
        editing={editing}
        onCancelReply={onCancelReply}
        onCancelEdit={onCancelEdit}
        onEditSubmit={onEditSubmit}
        onDraftChange={onDraftChange}
        onSend={onSend}
        onUpload={onUpload}
        onTypingInput={onTypingInput}
      />
    </>
  );
}

export function ChatPanel({ chat, className, ...rest }: ChatPanelProps) {
  return (
    <main
      className={cn(
        'h-full min-h-0 flex-1 flex-col bg-app',
        className,
      )}
    >
      {!chat ? (
        <EmptyChat />
      ) : (
        <ChatView chat={chat} {...rest} />
      )}
    </main>
  );
}
