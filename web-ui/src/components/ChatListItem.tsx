import { useEffect, useRef, useState } from 'react';
import {
  Archive,
  ArchiveRestore,
  BellOff,
  CheckCheck,
  MailOpen,
  MoreVertical,
  Pin,
  PinOff,
  Trash2,
} from 'lucide-react';
import type { ChatItem } from '../types';
import { avatarUrl } from '../api';
import { Avatar } from './ui/Avatar';
import { formatChatTime } from '../utils/format';
import { cn } from '../utils/classNames';

/** Срок мьюта, который понимает `PATCH /api/chats/{id}/state`. */
export type MuteDuration = '1h' | '8h' | 'forever';

interface ChatListItemProps {
  chat: ChatItem;
  active: boolean;
  onSelect: (chatId: number) => void;
  onTogglePin: (chatId: number, pinned: boolean) => void;
  onToggleArchive: (chatId: number, archived: boolean) => void;
  onMute: (chatId: number, duration: MuteDuration | null) => void;
  onMarkUnread: (chatId: number) => void;
  onMarkRead: (chatId: number) => void;
  onDelete: (chatId: number) => void;
}

interface MenuItemProps {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  danger?: boolean;
}

function MenuItem({ icon, label, onClick, danger }: MenuItemProps) {
  return (
    <button
      type="button"
      role="menuitem"
      onClick={onClick}
      className={cn(
        'flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-xs font-medium transition-colors',
        danger
          ? 'text-danger hover:bg-panel-strong'
          : 'text-ink hover:bg-panel-soft',
      )}
    >
      <span className="flex h-4 w-4 shrink-0 items-center justify-center" aria-hidden="true">
        {icon}
      </span>
      {label}
    </button>
  );
}

export function ChatListItem({
  chat,
  active,
  onSelect,
  onTogglePin,
  onToggleArchive,
  onMute,
  onMarkUnread,
  onMarkRead,
  onDelete,
}: ChatListItemProps) {
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);
  const unread = chat.unread_count > 0 || chat.marked_unread;
  const unreadLabel = chat.unread_count > 99 ? '99+' : String(chat.unread_count);

  useEffect(() => {
    if (!menuOpen) {
      return;
    }
    function handleClick(event: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        setMenuOpen(false);
      }
    }
    function handleKey(event: KeyboardEvent) {
      if (event.key === 'Escape') {
        setMenuOpen(false);
      }
    }
    document.addEventListener('mousedown', handleClick);
    document.addEventListener('keydown', handleKey);
    return () => {
      document.removeEventListener('mousedown', handleClick);
      document.removeEventListener('keydown', handleKey);
    };
  }, [menuOpen]);

  function run(action: () => void) {
    setMenuOpen(false);
    action();
  }

  return (
    <div
      className={cn(
        'group relative flex w-full items-center gap-3 rounded-2xl px-2 py-1 transition-colors',
        active ? 'bg-panel-strong' : 'hover:bg-panel-soft',
      )}
      onContextMenu={(event) => {
        event.preventDefault();
        setMenuOpen(true);
      }}
    >
      {active && (
        <span
          className="absolute left-0 top-1/2 h-6 w-0.5 -translate-y-1/2 rounded-full bg-accent"
          aria-hidden="true"
        />
      )}

      <button
        type="button"
        onClick={() => onSelect(chat.id)}
        aria-current={active ? 'true' : undefined}
        className="flex min-w-0 flex-1 items-center gap-3 py-1.5 text-left"
      >
        <Avatar
          title={chat.title}
          isGroup={chat.is_group}
          online={!chat.is_group && chat.is_online === true}
          src={
            !chat.is_group && chat.other_user_id
              ? avatarUrl(chat.other_user_id)
              : null
          }
          initialsFrom={chat.is_group ? null : chat.title}
        />

        <span className="min-w-0 flex-1">
          <span className="flex items-baseline gap-2">
            <span
              className={cn(
                'min-w-0 flex-1 truncate text-sm',
                // Непрочитанное и открытый чат выделяем жирнее — читаемость сверху вниз.
                active || unread ? 'font-semibold text-ink' : 'font-medium text-ink',
              )}
            >
              {chat.title}
            </span>
            {chat.muted && (
              <BellOff
                className="h-3 w-3 shrink-0 text-muted"
                aria-label="Уведомления отключены"
              />
            )}
            {chat.pinned && (
              <Pin
                className="h-3 w-3 shrink-0 text-muted"
                aria-label="Чат закреплён"
              />
            )}
            {chat.last_timestamp != null && chat.last_timestamp > 0 && (
              <span
                className={cn(
                  'shrink-0 text-[11px] tabular-nums',
                  unread ? 'font-medium text-accent-strong' : 'text-muted',
                )}
              >
                {formatChatTime(chat.last_timestamp)}
              </span>
            )}
          </span>
          <span className="mt-0.5 flex items-center gap-2">
            <span
              className={cn(
                'min-w-0 flex-1 truncate text-xs',
                unread ? 'font-medium text-ink' : 'text-muted',
              )}
            >
              {/* Медиа без подписи приходит с пустым text — показываем «Файл». */}
              {chat.last_message?.trim() ||
                (chat.last_timestamp != null && chat.last_timestamp > 0
                  ? 'Файл'
                  : 'Нет сообщений')}
            </span>
            {unread && (
              <span
                key={`unread-${chat.unread_count}-${chat.marked_unread}`}
                className="badge-pop shrink-0 rounded-full bg-accent px-1.5 py-0.5 text-[10px] font-semibold leading-none text-white"
                aria-label={
                  chat.marked_unread && chat.unread_count === 0
                    ? 'Помечен непрочитанным'
                    : `Непрочитанных сообщений: ${chat.unread_count}`
                }
              >
                {chat.marked_unread && chat.unread_count === 0 ? '•' : unreadLabel}
              </span>
            )}
          </span>
        </span>
      </button>

      <button
        type="button"
        onClick={() => setMenuOpen((open) => !open)}
        aria-label="Действия с чатом"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        className={cn(
          'flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-muted transition-opacity',
          'hover:bg-panel-strong hover:text-ink focus-visible:opacity-100',
          menuOpen ? 'opacity-100' : 'opacity-0 group-hover:opacity-100',
        )}
      >
        <MoreVertical className="h-4 w-4" aria-hidden="true" />
      </button>

      {menuOpen && (
        <div
          ref={menuRef}
          role="menu"
          className="animate-fade-in-up absolute right-2 top-full z-20 mt-1 w-56 rounded-xl border border-line bg-panel p-1 shadow-2xl"
        >
          <MenuItem
            icon={
              chat.pinned ? (
                <PinOff className="h-3.5 w-3.5" />
              ) : (
                <Pin className="h-3.5 w-3.5" />
              )
            }
            label={chat.pinned ? 'Открепить' : 'Закрепить'}
            onClick={() => run(() => onTogglePin(chat.id, !chat.pinned))}
          />

          {chat.muted ? (
            <MenuItem
              icon={<BellOff className="h-3.5 w-3.5" />}
              label="Включить уведомления"
              onClick={() => run(() => onMute(chat.id, null))}
            />
          ) : (
            <>
              <MenuItem
                icon={<BellOff className="h-3.5 w-3.5" />}
                label="Без звука на 1 час"
                onClick={() => run(() => onMute(chat.id, '1h'))}
              />
              <MenuItem
                icon={<BellOff className="h-3.5 w-3.5" />}
                label="Без звука на 8 часов"
                onClick={() => run(() => onMute(chat.id, '8h'))}
              />
              <MenuItem
                icon={<BellOff className="h-3.5 w-3.5" />}
                label="Без звука навсегда"
                onClick={() => run(() => onMute(chat.id, 'forever'))}
              />
            </>
          )}

          <MenuItem
            icon={
              chat.archived ? (
                <ArchiveRestore className="h-3.5 w-3.5" />
              ) : (
                <Archive className="h-3.5 w-3.5" />
              )
            }
            label={chat.archived ? 'Вернуть из архива' : 'В архив'}
            onClick={() => run(() => onToggleArchive(chat.id, !chat.archived))}
          />

          {unread ? (
            <MenuItem
              icon={<CheckCheck className="h-3.5 w-3.5" />}
              label="Пометить прочитанным"
              onClick={() => run(() => onMarkRead(chat.id))}
            />
          ) : (
            <MenuItem
              icon={<MailOpen className="h-3.5 w-3.5" />}
              label="Пометить непрочитанным"
              onClick={() => run(() => onMarkUnread(chat.id))}
            />
          )}

          <div className="my-1 border-t border-line" aria-hidden="true" />

          <MenuItem
            danger
            icon={<Trash2 className="h-3.5 w-3.5" />}
            label={chat.is_group ? 'Покинуть группу' : 'Удалить чат'}
            onClick={() => run(() => onDelete(chat.id))}
          />
        </div>
      )}
    </div>
  );
}
