import type { ChatItem } from '../types';
import { Avatar } from './ui/Avatar';
import { formatChatTime } from '../utils/format';
import { cn } from '../utils/classNames';

interface ChatListItemProps {
  chat: ChatItem;
  active: boolean;
  onSelect: (chatId: number) => void;
}

export function ChatListItem({ chat, active, onSelect }: ChatListItemProps) {
  const unread = chat.unread_count > 0;
  const unreadLabel = chat.unread_count > 99 ? '99+' : String(chat.unread_count);

  return (
    <button
      type="button"
      onClick={() => onSelect(chat.id)}
      aria-current={active ? 'true' : undefined}
      className={cn(
        'group relative flex w-full items-center gap-3 rounded-2xl px-3 py-2.5 text-left transition-colors',
        active
          ? 'bg-accent-soft ring-1 ring-accent/25'
          : 'hover:bg-panel-soft',
      )}
    >
      <Avatar
        title={chat.title}
        isGroup={chat.is_group}
        online={!chat.is_group && chat.is_online === true}
      />

      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2">
          <span
            className={cn(
              'min-w-0 flex-1 truncate text-sm',
              active ? 'font-semibold text-ink' : 'font-medium text-ink',
            )}
          >
            {chat.title}
          </span>
          {chat.last_timestamp != null && chat.last_timestamp > 0 && (
            <span className="shrink-0 text-[11px] tabular-nums text-muted">
              {formatChatTime(chat.last_timestamp)}
            </span>
          )}
        </span>
        <span className="mt-0.5 flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-xs text-muted">
            {chat.last_message || 'Нет сообщений'}
          </span>
          {unread && (
            <span
              className="shrink-0 rounded-full bg-accent px-1.5 py-0.5 text-[10px] font-semibold leading-none text-white"
              aria-label={`Непрочитанных сообщений: ${chat.unread_count}`}
            >
              {unreadLabel}
            </span>
          )}
        </span>
      </span>
    </button>
  );
}
