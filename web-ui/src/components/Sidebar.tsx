import { useMemo, useState } from 'react';
import {
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
import { ChatListItem } from './ChatListItem';
import { cn } from '../utils/classNames';

interface SidebarProps {
  me: Me | null;
  fallbackPhone: string;
  chats: ChatItem[];
  loading: boolean;
  activeChatId: number | null;
  wsStatus: WsStatus;
  theme: Theme;
  onToggleTheme: () => void;
  onSelectChat: (chatId: number) => void;
  onNewChat: () => void;
  onNewGroup: () => void;
  onOpenProfile: () => void;
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
  onToggleTheme,
  onSelectChat,
  onNewChat,
  onNewGroup,
  onOpenProfile,
  className,
}: SidebarProps) {
  const [search, setSearch] = useState('');
  const phone = me?.phone ?? fallbackPhone;

  const filteredChats = useMemo(() => {
    const query = search.trim().toLowerCase();
    if (!query) {
      return chats;
    }
    return chats.filter((chat) => {
      const titleMatch = chat.title.toLowerCase().includes(query);
      const messageMatch = (chat.last_message ?? '').toLowerCase().includes(query);
      return titleMatch || messageMatch;
    });
  }, [chats, search]);

  const status = WS_STATUS[wsStatus];

  return (
    <aside
      className={cn(
        'h-full min-h-0 w-full flex-col border-r border-line bg-panel/70 backdrop-blur-xl md:w-[360px] md:shrink-0 lg:w-[400px]',
        className,
      )}
    >
      <header className="flex items-center justify-between gap-2 px-4 pb-4 pt-5">
        <div className="flex min-w-0 items-center gap-2.5">
          <span className="logo-glow flex h-10 w-10 shrink-0 items-center justify-center rounded-2xl bg-gradient-to-br from-indigo-500 to-violet-600 text-white">
            <MessageCircle className="h-5 w-5" strokeWidth={2.3} aria-hidden="true" />
          </span>
          <span className="min-w-0">
            <span className="brand-text block text-xl font-bold leading-tight tracking-tight">
              KoSta
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
        ) : filteredChats.length > 0 ? (
          <div className="space-y-0.5">
            {filteredChats.map((chat) => (
              <ChatListItem
                key={chat.id}
                chat={chat}
                active={chat.id === activeChatId}
                onSelect={onSelectChat}
              />
            ))}
          </div>
        ) : (
          <div className="flex h-full flex-col items-center justify-center px-6 text-center">
            <span className="mb-3 flex h-12 w-12 items-center justify-center rounded-2xl bg-panel-soft text-muted">
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
        <span className="text-[11px] text-muted" title={phone}>
          {phone || 'KoSta'}
        </span>
      </footer>
    </aside>
  );
}
