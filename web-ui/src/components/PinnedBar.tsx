import { Pin, X } from 'lucide-react';
import type { Participant, UiMessage } from '../types';
import { messagePreview } from '../utils/media';
import { userLabel } from '../utils/format';

interface PinnedBarProps {
  message: UiMessage;
  participants: Participant[];
  /** Может ли текущий пользователь снять закрепление. */
  canUnpin: boolean;
  onUnpin: () => void;
}

/** Плашка закреплённого сообщения под шапкой чата. */
export function PinnedBar({
  message,
  participants,
  canUnpin,
  onUnpin,
}: PinnedBarProps) {
  const author = participants.find(
    (participant) => participant.user_id === message.sender_id,
  );

  return (
    <div className="flex items-center gap-2 border-b border-line bg-panel px-3 py-1.5 md:px-5">
      <span className="h-8 w-0.5 shrink-0 rounded-full bg-accent" aria-hidden="true" />
      <Pin className="h-3.5 w-3.5 shrink-0 text-accent-strong" aria-hidden="true" />
      <div className="min-w-0 flex-1">
        <p className="text-[11px] font-semibold leading-tight text-accent-strong">
          Закреплённое сообщение
        </p>
        <p className="truncate text-xs leading-tight text-muted">
          {author ? `${userLabel(author)}: ` : ''}
          {messagePreview(message)}
        </p>
      </div>
      {canUnpin && (
        <button
          type="button"
          onClick={onUnpin}
          aria-label="Открепить сообщение"
          title="Открепить"
          className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg text-muted transition-colors hover:bg-panel-soft hover:text-ink"
        >
          <X className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      )}
    </div>
  );
}