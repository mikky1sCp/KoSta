import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from 'react';
import { MessageSquare, Search, UserRound, X } from 'lucide-react';
import type { Api } from '../api';
import { avatarUrl } from '../api';
import type { ChatItem, GlobalSearchHit, UserSearchItem } from '../types';
import { getErrorMessage } from '../utils/errors';
import { cn } from '../utils/classNames';
import { Avatar } from './ui/Avatar';
import { Spinner } from './ui/Spinner';

/** Сколько результатов показывать в каждой группе. */
const GROUP_LIMIT = 6;
const SEARCH_DEBOUNCE_MS = 250;

type PaletteItem =
  | { kind: 'message'; hit: GlobalSearchHit }
  | { kind: 'user'; user: UserSearchItem }
  | { kind: 'chat'; chat: ChatItem };

interface SearchPaletteProps {
  api: Api;
  chats: ChatItem[];
  onClose: () => void;
  onOpenChat: (chatId: number) => void;
  onOpenMessage: (chatId: number, messageId: number) => void;
  onStartChat: (user: UserSearchItem) => void;
}

function itemKey(item: PaletteItem): string {
  switch (item.kind) {
    case 'message':
      return `message-${item.hit.chat_id}-${item.hit.message.id}`;
    case 'user':
      return `user-${item.user.user_id}`;
    case 'chat':
      return `chat-${item.chat.id}`;
  }
}

function messagePreview(hit: GlobalSearchHit): string {
  const text = hit.message.text.trim();
  if (text.length > 0) {
    return text;
  }
  return hit.message.is_media ? 'Вложение' : 'Пустое сообщение';
}

/**
 * Палитра глобального поиска: сообщения во всех чатах, люди по @username и
 * локальные чаты. Открывается по Ctrl+K, закрывается по Esc.
 */
export function SearchPalette({
  api,
  chats,
  onClose,
  onOpenChat,
  onOpenMessage,
  onStartChat,
}: SearchPaletteProps) {
  const [query, setQuery] = useState('');
  const [messages, setMessages] = useState<GlobalSearchHit[]>([]);
  const [users, setUsers] = useState<UserSearchItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  useEffect(() => {
    function handleKeyDown(event: globalThis.KeyboardEvent) {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
      }
    }
    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [onClose]);

  const chatMatches = useMemo(() => {
    const needle = query.trim().toLowerCase();
    if (!needle) {
      return [];
    }
    return chats
      .filter(
        (chat) =>
          chat.title.toLowerCase().includes(needle) ||
          (chat.last_message ?? '').toLowerCase().includes(needle),
      )
      .slice(0, GROUP_LIMIT);
  }, [chats, query]);

  useEffect(() => {
    const needle = query.trim();
    if (needle.length === 0) {
      setMessages([]);
      setUsers([]);
      setLoading(false);
      setError('');
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError('');
    const timer = window.setTimeout(() => {
      void Promise.all([
        api.searchGlobal(needle).catch(() => []),
        api.searchUsers(needle).catch(() => []),
      ])
        .then(([messageHits, userHits]) => {
          if (cancelled) {
            return;
          }
          setMessages(messageHits);
          setUsers(userHits);
          setLoading(false);
        })
        .catch((e) => {
          if (cancelled) {
            return;
          }
          setLoading(false);
          setError(getErrorMessage(e));
        });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      window.clearTimeout(timer);
    };
  }, [query, api]);

  const items = useMemo<PaletteItem[]>(() => {
    const list: PaletteItem[] = [];
    for (const hit of messages.slice(0, GROUP_LIMIT)) {
      list.push({ kind: 'message', hit });
    }
    for (const user of users.slice(0, GROUP_LIMIT)) {
      list.push({ kind: 'user', user });
    }
    for (const chat of chatMatches.slice(0, GROUP_LIMIT)) {
      list.push({ kind: 'chat', chat });
    }
    return list;
  }, [messages, users, chatMatches]);

  useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  useEffect(() => {
    const element = listRef.current?.querySelector<HTMLElement>(
      `[data-index="${activeIndex}"]`,
    );
    element?.scrollIntoView({ block: 'nearest' });
  }, [activeIndex]);

  function activate(index: number) {
    const item = items[index];
    if (!item) {
      return;
    }
    onClose();
    if (item.kind === 'message') {
      onOpenMessage(item.hit.chat_id, item.hit.message.id);
    } else if (item.kind === 'user') {
      onStartChat(item.user);
    } else {
      onOpenChat(item.chat.id);
    }
  }

  function handleInputKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      setActiveIndex((current) => Math.min(items.length - 1, current + 1));
    } else if (event.key === 'ArrowUp') {
      event.preventDefault();
      setActiveIndex((current) => Math.max(0, current - 1));
    } else if (event.key === 'Enter') {
      event.preventDefault();
      activate(activeIndex);
    }
  }

  const hasQuery = query.trim().length > 0;
  const messageStart = 0;
  const userStart = messages.slice(0, GROUP_LIMIT).length;
  const chatStart = userStart + users.slice(0, GROUP_LIMIT).length;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center bg-[var(--overlay)] px-4 pt-[10vh] sm:pt-[14vh]"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label="Поиск по сообщениям, людям и чатам"
        className="animate-fade-in-up flex max-h-[72vh] w-full max-w-xl flex-col overflow-hidden rounded-2xl border border-line bg-panel shadow-2xl"
      >
        <div className="flex items-center gap-2 border-b border-line px-4 py-2.5">
          <Search className="h-4 w-4 shrink-0 text-muted" aria-hidden="true" />
          <input
            ref={inputRef}
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={handleInputKeyDown}
            placeholder="Сообщения, люди, чаты…"
            aria-label="Поиск"
            className="h-10 w-full bg-transparent text-sm text-ink outline-none placeholder:text-muted"
          />
          {loading ? (
            <Spinner className="h-4 w-4" label="Поиск" />
          ) : (
            hasQuery && (
              <button
                type="button"
                onClick={() => setQuery('')}
                aria-label="Очистить поиск"
                className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-muted transition-colors hover:bg-panel-strong hover:text-ink"
              >
                <X className="h-3.5 w-3.5" aria-hidden="true" />
              </button>
            )
          )}
          <kbd className="hidden shrink-0 rounded-md border border-line bg-panel-soft px-1.5 py-0.5 font-sans text-[10px] font-medium text-muted sm:inline">
            Esc
          </kbd>
        </div>

        <div ref={listRef} className="kosta-scroll min-h-0 flex-1 overflow-y-auto py-2">
          {!hasQuery ? (
            <div className="px-4 py-8 text-center text-sm leading-relaxed text-muted">
              Начните вводить — найдём сообщения во всех чатах, людей по
              @username и ваши чаты.
            </div>
          ) : items.length === 0 && !loading ? (
            <p className="px-4 py-8 text-center text-sm text-muted">
              Ничего не найдено.
            </p>
          ) : (
            <div className="space-y-1">
              {messages.length > 0 && (
                <p className="px-3 pb-1 pt-2 text-[11px] font-medium uppercase tracking-wide text-muted">
                  Сообщения
                </p>
              )}
              {messages.slice(0, GROUP_LIMIT).map((hit, index) => {
                const flatIndex = messageStart + index;
                return (
                  <button
                    key={itemKey({ kind: 'message', hit })}
                    type="button"
                    data-index={flatIndex}
                    onMouseEnter={() => setActiveIndex(flatIndex)}
                    onClick={() => activate(flatIndex)}
                    className={cn(
                      'flex w-full items-start gap-3 rounded-lg px-3 py-2 text-left transition-colors',
                      flatIndex === activeIndex ? 'bg-panel-strong' : 'hover:bg-panel-soft',
                    )}
                  >
                    <MessageSquare
                      className="mt-0.5 h-4 w-4 shrink-0 text-accent-strong"
                      aria-hidden="true"
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium text-ink">
                        {hit.chat_title || 'Чат'}
                      </span>
                      <span className="block truncate text-xs text-muted">
                        {messagePreview(hit)}
                      </span>
                    </span>
                  </button>
                );
              })}

              {users.length > 0 && (
                <p className="px-3 pb-1 pt-2 text-[11px] font-medium uppercase tracking-wide text-muted">
                  Люди
                </p>
              )}
              {users.slice(0, GROUP_LIMIT).map((user, index) => {
                const flatIndex = userStart + index;
                return (
                  <button
                    key={itemKey({ kind: 'user', user })}
                    type="button"
                    data-index={flatIndex}
                    onMouseEnter={() => setActiveIndex(flatIndex)}
                    onClick={() => activate(flatIndex)}
                    className={cn(
                      'flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left transition-colors',
                      flatIndex === activeIndex ? 'bg-panel-strong' : 'hover:bg-panel-soft',
                    )}
                  >
                    <Avatar
                      title={user.display_name ?? user.username ?? 'Пользователь'}
                      isGroup={false}
                      online={user.is_online}
                      size="sm"
                      src={user.has_avatar ? avatarUrl(user.user_id) : null}
                      initialsFrom={user.display_name ?? null}
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium text-ink">
                        {user.display_name ?? user.username ?? 'Без имени'}
                      </span>
                      <span className="block truncate text-xs text-muted">
                        {user.username ? `@${user.username}` : 'Пользователь'}
                        {user.is_online ? ' · онлайн' : ''}
                      </span>
                    </span>
                    <UserRound className="h-4 w-4 shrink-0 text-muted" aria-hidden="true" />
                  </button>
                );
              })}

              {chatMatches.length > 0 && (
                <p className="px-3 pb-1 pt-2 text-[11px] font-medium uppercase tracking-wide text-muted">
                  Чаты
                </p>
              )}
              {chatMatches.slice(0, GROUP_LIMIT).map((chat, index) => {
                const flatIndex = chatStart + index;
                return (
                  <button
                    key={itemKey({ kind: 'chat', chat })}
                    type="button"
                    data-index={flatIndex}
                    onMouseEnter={() => setActiveIndex(flatIndex)}
                    onClick={() => activate(flatIndex)}
                    className={cn(
                      'flex w-full items-center gap-3 rounded-lg px-3 py-2 text-left transition-colors',
                      flatIndex === activeIndex ? 'bg-panel-strong' : 'hover:bg-panel-soft',
                    )}
                  >
                    <Search className="h-4 w-4 shrink-0 text-muted" aria-hidden="true" />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium text-ink">
                        {chat.title}
                      </span>
                      <span className="block truncate text-xs text-muted">
                        {chat.last_message ?? 'Чат'}
                      </span>
                    </span>
                  </button>
                );
              })}
            </div>
          )}

          {error && (
            <p role="alert" className="px-4 py-2 text-xs text-danger">
              {error}
            </p>
          )}
        </div>

        <div className="flex items-center justify-between border-t border-line px-4 py-2 text-[11px] text-muted">
          <span>↑↓ — выбор · Enter — открыть · Esc — закрыть</span>
          <span className="hidden sm:inline">Ctrl+K</span>
        </div>
      </div>
    </div>
  );
}
