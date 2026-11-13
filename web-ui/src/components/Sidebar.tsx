import { useMemo, useState, type Ref } from 'react';
import {
  Archive,
  MessageCircle,
  MessageSquarePlus,
  Moon,
  Search,
  Sun,
  UserRound,
  UsersRound,
  X,
} from 'lucide-react';
import type { ChatItem, Me, Theme, WsStatus } from '../types';
import { ChatListItem, type MuteDuration } from './ChatListItem';
import { cn } from '../utils/classNames';

interface SidebarProps {
  me: Me | null;
  fallbackPhone: string;
  chats: ChatItem[];
  loading: boolean;
  activeChatId: number | null;
  wsStatus: WsStatus;
  theme: Theme;
  /** Ref на поле поиска — для глобальных шорткатов (Ctrl+K, «/»). */
  searchInputRef?: Ref<HTMLInputElement>;
  onToggleTheme: () => void;
  onSelectChat: (chatId: number) => void;
  onNewChat: () => void;
  onNewGroup: () => void;
  onOpenProfile: () => void;
  onTogglePin: (chatId: number, pinned: boolean) => void;
  onToggleArchive: (chatId: number, archived: boolean) => void;
  onMute: (chatId: number, duration: MuteDuration | null) => void;
  onMarkUnread: (chatId: number) => void;
  onMarkRead: (chatId: number) => void;
  onDeleteChat: (chatId: number) => void;
  className?: string;
}

const WS_STATUS: Record<
  WsStatus,
  { label: string; dotClass: string; textClass: string }
> = {
  connected: {
    label: 'Подключено',
    dotClass: 'bg-online status-dot-online',
    textClass: 'text-muted',
  },
  connecting: {
    label: 'Соединение…',
    dotClass: 'bg-warn',
    textClass: 'text-warn',
  },
  reconnecting: {
    label: 'Переподключение…',
    dotClass: 'bg-warn',
    textClass: 'text-warn',
  },
  offline: {
    label: 'Нет соединения',
    dotClass: 'bg-danger',
    textClass: 'text-danger',
  },
};

function ChatListSkeleton() {
  return (
    <div className="space-y-1 px-2 py-2" aria-hidden="true">
      {Array.from({ length: 6 }, (_, index) => (
        <div key={index} className="flex items-center gap-3 rounded-2xl px-3 py-2.5">
          <div className="h-11 w-11 shrink-0 animate-pulse rounded-2xl bg-panel-strong" />
          <div className="flex-1 space-y-2">
            <div className="h-3 w-2/3 animate-pulse rounded-full bg-panel-strong" />
            <div className="h-2.5 w-full animate-pulse rounded-full bg-panel-strong/70" />
          </div>
        </div>
      ))}
    </div>
  );
}

export function Sidebar({
  me,
  fallbackPhone,
  chats,
  loading,
  activeChatId,
  wsStatus,
  theme,
  searchInputRef,
  onToggleTheme,
  onSelectChat,
  onNewChat,
  onNewGroup,
  onOpenProfile,
  onTogglePin,
  onToggleArchive,
  onMute,
  onMarkUnread,
  onMarkRead,
  onDeleteChat,
  className,
}: SidebarProps) {
  const [search, setSearch] = useState('');
  const [showArchive, setShowArchive] = useState(false);
  const phone = me?.phone ?? fallbackPhone;
  const selfLabel = me?.display_name?.trim() || phone;
  const query = search.trim().toLowerCase();

  const matches = (chat: ChatItem) =>
    !query ||
    chat.title.toLowerCase().includes(query) ||
    (chat.last_message ?? '').toLowerCase().includes(query);

  // Закреплённые всегда сверху — внутри активных и внутри архива.
  const sortForList = (list: ChatItem[]) =>
    [...list].sort((left, right) => {
      if (left.pinned !== right.pinned) {
        return left.pinned ? -1 : 1;
      }
      return (right.last_timestamp ?? 0) - (left.last_timestamp ?? 0);
    });

  const activeChats = useMemo(
    () => sortForList(chats.filter((chat) => !chat.archived && matches(chat))),
    // matches зависит только от query и chats.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [chats, query],
  );
  const archivedChats = useMemo(
    () => sortForList(chats.filter((chat) => chat.archived && matches(chat))),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [chats, query],
  );
  // При поиске архивные чаты показываем сразу, без сворачивания.
  const archiveVisible = query.length > 0 || showArchive;

  const renderChat = (chat: ChatItem) => (
    <ChatListItem
      key={chat.id}
      chat={chat}
      active={chat.id === activeChatId}
      onSelect={onSelectChat}
      onTogglePin={onTogglePin}
      onToggleArchive={onToggleArchive}
      onMute={onMute}
      onMarkUnread={onMarkUnread}
      onMarkRead={onMarkRead}
      onDelete={onDeleteChat}
    />
  );

  const status = WS_STATUS[wsStatus];

  return (
    <aside
      className={cn(
        'h-full min-h-0 w-full flex-col border-r border-line bg-panel md:w-[360px] md:shrink-0 lg:w-[400px]',
        className,
      )}
    >
      <header className="flex items-center justify-between gap-2 px-4 pb-4 pt-5">
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="flex h-10 w-10 shrink-0 items-center justify-center rounded-2xl bg-accent text-white">
            <MessageCircle className="h-5 w-5" strokeWidth={2.3} aria-hidden="true" />
          </span>
          <span className="min-w-0">
            <span className="brand-text block text-xl font-bold leading-tight tracking-tight">
              KoSta<span className="text-accent-strong">.</span>
            </span>
            <span className="block text-[10px] uppercase tracking-[0.18em] text-muted">
              messenger
            </span>
          </span>
        </div>

        <div className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            onClick={onToggleTheme}
            aria-label={
              theme === 'dark' ? 'Включить светлую тему' : 'Включить тёмную тему'
            }
            title={theme === 'dark' ? 'Светлая тема' : 'Тёмная тема'}
            className="flex h-9 w-9 items-center justify-center rounded-xl text-muted transition-colors hover:bg-panel-soft hover:text-ink"
          >
            {theme === 'dark' ? (
              <Sun className="h-4 w-4" aria-hidden="true" />
            ) : (
              <Moon className="h-4 w-4" aria-hidden="true" />
            )}
          </button>
          <button
            type="button"
            onClick={onOpenProfile}
            title={phone || 'Профиль'}
            aria-label="Открыть профиль"
            className="flex h-9 w-9 items-center justify-center rounded-xl bg-panel-soft text-muted transition-colors hover:bg-panel-strong hover:text-ink"
          >
            <UserRound className="h-4 w-4" aria-hidden="true" />
          </button>
        </div>
      </header>

      <div className="px-3 pb-3">
        <div className="flex items-center gap-2 rounded-xl border border-line bg-panel-soft px-3 transition-colors focus-within:border-accent/60">
          <Search className="h-4 w-4 shrink-0 text-muted" aria-hidden="true" />
          <input
            ref={searchInputRef}
            type="search"
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Поиск по чатам"
            aria-label="Поиск по чатам"
            className="h-10 w-full bg-transparent text-sm text-ink outline-none placeholder:text-muted"
          />
          {search.length > 0 && (
            <button
              type="button"
              onClick={() => setSearch('')}
              aria-label="Очистить поиск"
              className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-muted transition-colors hover:bg-panel-strong hover:text-ink"
            >
              <X className="h-3.5 w-3.5" aria-hidden="true" />
            </button>
          )}
        </div>
      </div>

      <div className="grid grid-cols-2 gap-2 px-3 pb-3">
        <button
          type="button"
          onClick={onNewChat}
          className="flex h-9 items-center justify-center gap-1.5 rounded-xl border border-line bg-panel-soft px-2 text-xs font-medium text-ink transition-colors hover:bg-panel-strong"
        >
          <MessageSquarePlus className="h-4 w-4 text-accent-strong" aria-hidden="true" />
          Новый чат
        </button>
        <button
          type="button"
          onClick={onNewGroup}
          className="flex h-9 items-center justify-center gap-1.5 rounded-xl border border-line bg-panel-soft px-2 text-xs font-medium text-ink transition-colors hover:bg-panel-strong"
        >
          <UsersRound className="h-4 w-4 text-accent-strong" aria-hidden="true" />
          Новая группа
        </button>
      </div>

      <div className="kosta-scroll min-h-0 flex-1 overflow-y-auto px-2 pb-2">
        {loading ? (
          <ChatListSkeleton />
        ) : activeChats.length > 0 || archivedChats.length > 0 ? (
          <div className="space-y-0.5">
            {activeChats.map(renderChat)}

            {archivedChats.length > 0 && (
              <button
                type="button"
                onClick={() => setShowArchive((open) => !open)}
                aria-expanded={showArchive}
                className="mt-2 flex w-full items-center gap-2 rounded-xl px-3 py-2 text-xs font-medium text-muted transition-colors hover:bg-panel-soft hover:text-ink"
              >
                <Archive className="h-3.5 w-3.5" aria-hidden="true" />
                <span className="flex-1 text-left">Архив</span>
                <span className="tabular-nums">{archivedChats.length}</span>
              </button>
            )}
            {archivedChats.length > 0 && archiveVisible && (
              <div className="space-y-0.5">{archivedChats.map(renderChat)}</div>
            )}
          </div>
        ) : (
          <div className="flex h-full flex-col items-center justify-center px-6 text-center">
            <span className="mb-3 flex h-12 w-12 items-center justify-center rounded-2xl bg-accent-soft text-accent-strong">
              {search.trim() ? (
                <Search className="h-5 w-5" aria-hidden="true" />
              ) : (
                <MessageSquarePlus className="h-5 w-5" aria-hidden="true" />
              )}
            </span>
            <p className="text-sm font-medium text-ink">
              {search.trim() ? 'Ничего не найдено' : 'Пока нет чатов'}
            </p>
            <p className="mt-1 text-xs text-muted">
              {search.trim()
                ? 'Попробуйте другой запрос'
                : 'Начните личный чат или создайте группу'}
            </p>
          </div>
        )}
      </div>

      <footer className="flex items-center justify-between gap-2 border-t border-line px-4 py-3">
        <div className="flex items-center gap-2 text-xs">
          <span
            className={cn('h-2 w-2 rounded-full', status.dotClass)}
            aria-hidden="true"
          />
          <span className={status.textClass}>{status.label}</span>
        </div>
        <span className="truncate text-[11px] text-muted" title={phone}>
          {selfLabel || 'KoSta'}
        </span>
      </footer>
    </aside>
  );
}
