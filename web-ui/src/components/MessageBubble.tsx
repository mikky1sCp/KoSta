import { Check, CheckCheck, Clock3 } from 'lucide-react';
import type { UiMessage } from '../types';
import { formatTime } from '../utils/format';
import { cn } from '../utils/classNames';

interface OutgoingStatusProps {
  message: UiMessage;
}

function OutgoingStatus({ message }: OutgoingStatusProps) {
  if (message.pending) {
    return <Clock3 className="h-3 w-3" aria-label="Отправляется" />;
  }
  if (message.read) {
    return <CheckCheck className="h-3.5 w-3.5 text-sky-200" aria-label="Прочитано" />;
  }
  if (message.delivered) {
    return <CheckCheck className="h-3.5 w-3.5 opacity-70" aria-label="Доставлено" />;
  }
  return <Check className="h-3.5 w-3.5 opacity-70" aria-label="Отправлено" />;
}

interface MessageBubbleProps {
  message: UiMessage;
  isOwn: boolean;
  showSender: boolean;
  senderPhone: string | null;
  tail: boolean;
  inGroup: boolean;
}

export function MessageBubble({
  message,
  isOwn,
  showSender,
  senderPhone,
  tail,
  inGroup,
}: MessageBubbleProps) {
  return (
    <div
      className={cn(
        'animate-fade-in-up flex w-full',
        isOwn ? 'justify-end' : 'justify-start',
      )}
    >
      <div className="flex max-w-[86%] flex-col md:max-w-[72%]">
        {inGroup && !isOwn && showSender && senderPhone && (
          <span className="mb-1 ml-1.5 truncate text-[11px] font-semibold text-accent-strong">
            {senderPhone}
          </span>
        )}

        <div
          className={cn(
            'rounded-2xl px-3.5 py-2 shadow-sm',
            isOwn ? 'bubble-out' : 'bubble-in',
            tail && (isOwn ? 'rounded-br-md' : 'rounded-bl-md'),
            message.pending && 'opacity-80',
          )}
        >
          <p className="whitespace-pre-wrap break-words text-sm leading-relaxed">
            {message.text}
          </p>
          <div
            className={cn(
              'mt-1 flex items-center justify-end gap-1 text-[10px] leading-none',
              isOwn ? 'text-white/75' : 'text-muted',
            )}
          >
            <span className="tabular-nums">{formatTime(message.timestamp)}</span>
            {isOwn && <OutgoingStatus message={message} />}
          </div>
        </div>
      </div>
    </div>
  );
}
