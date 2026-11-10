import { ArrowLeft, Info, Search, X } from 'lucide-react';
import type { ChatDetails, ChatItem } from '../types';
import { avatarUrl } from '../api';
import { Avatar } from './ui/Avatar';
import { pluralize } from '../utils/format';
import { cn } from '../utils/classNames';

interface ChatHeaderProps {
  chat: ChatItem;
  details: ChatDetails | null;
  typingLabel: string | null;
  searchOpen: boolean;
  onToggleSearch: () => void;
  onBack: () => void;
  onOpenInfo: () => void;
}

export function ChatHeader({
  chat,
  details,
  typingLabel,
  searchOpen,
  onToggleSearch,
  onBack,
  onOpenInfo,
}: ChatHeaderProps) {
  const online = !chat.is_group && chat.is_online === true;
  const participantsCount = details?.participants.length ?? null;

  let subtitle: string;
  if (typingLabel) {
    subtitle = typingLabel;
  } else if (chat.is_group) {
    subtitle =
      participantsCount !== null
        ? `${participantsCount} ${pluralize(
            participantsCount,
            'участник',
            'участника',
            'участников',
          )}`
        : 'Группа';
  } else {
    subtitle = online ? 'онлайн' : 'не в сети';
  }

  return (
    <header className="flex items-center gap-3 border-b border-line bg-panel px-3 py-3 md:px-5">
      <button
        type="button"
        onClick={onBack}
        aria-label="Назад к списку чатов"
        className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-muted transition-colors hover:bg-panel-soft hover:text-ink md:hidden"
      >
        <ArrowLeft className="h-4 w-4" aria-hidden="true" />
      </button>

      <Avatar
        title={chat.title}
        isGroup={chat.is_group}
        online={online}
        size="md"
        src={
          !chat.is_group && chat.other_user_id
            ? avatarUrl(chat.other_user_id)
            : null
        }
        initialsFrom={chat.is_group ? null : chat.title}
      />

      <div className="min-w-0 flex-1">
        <h2 className="truncate text-sm font-semibold tracking-tight text-ink md:text-base">
          {chat.title}
        </h2>
        <p
          className={cn(
            'truncate text-xs',
            typingLabel
              ? 'text-accent-strong'
              : online
                ? 'text-online'
                : 'text-muted',
          )}
        >
          {subtitle}
        </p>
      </div>

      <button
        type="button"
        onClick={onToggleSearch}
        aria-label={searchOpen ? 'Закрыть поиск' : 'Поиск по сообщениям'}
        title={searchOpen ? 'Закрыть поиск' : 'Поиск по сообщениям'}
        aria-pressed={searchOpen}
        className={cn(
          'flex h-9 w-9 shrink-0 items-center justify-center rounded-xl transition-colors hover:bg-panel-soft hover:text-ink',
          searchOpen ? 'bg-panel-strong text-ink' : 'text-muted',
        )}
      >
        {searchOpen ? (
          <X className="h-4 w-4" aria-hidden="true" />
        ) : (
          <Search className="h-4 w-4" aria-hidden="true" />
        )}
      </button>

      <button
        type="button"
        onClick={onOpenInfo}
        aria-label="Информация о чате"
        title="Информация о чате"
        className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-muted transition-colors hover:bg-panel-soft hover:text-ink"
      >
        <Info className="h-4 w-4" aria-hidden="true" />
      </button>
    </header>
  );
}
