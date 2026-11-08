import { Check, CheckCheck, Clock3, Copy } from 'lucide-react';
import type { Api } from '../api';
import type { UiMessage } from '../types';
import { formatTime } from '../utils/format';
import { isImageMimeType } from '../utils/media';
import { cn } from '../utils/classNames';
import { FileAttachment } from './media/FileAttachment';
import { MediaImage } from './media/MediaImage';

interface OutgoingStatusProps {
  message: UiMessage;
}

function OutgoingStatus({ message }: OutgoingStatusProps) {
  if (message.pending) {
    return <Clock3 className="h-3 w-3" aria-label="Отправляется" />;
  }
  if (message.read) {
    return <CheckCheck className="h-3.5 w-3.5 text-white/90" aria-label="Прочитано" />;
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
  /** Устойчивый цвет имени отправителя в группе (есть у каждого участника). */
  senderColor?: string;
  tail: boolean;
  inGroup: boolean;
  api: Api;
  onOpenImage: (message: UiMessage, url: string) => void;
  onDownload: (message: UiMessage) => Promise<void>;
  onCopyMessage: (message: UiMessage) => void;
}

export function MessageBubble({
  message,
  isOwn,
  showSender,
  senderPhone,
  senderColor,
  tail,
  inGroup,
  api,
  onOpenImage,
  onDownload,
  onCopyMessage,
}: MessageBubbleProps) {
  const hasMedia = message.is_media === true;
  const hasImage = hasMedia && isImageMimeType(message.media_type);
  const caption = message.text.trim();
  const showCaption = caption.length > 0;

  return (
    <div
      className={cn(
        'animate-fade-in-up group flex w-full',
        isOwn ? 'justify-end' : 'justify-start',
      )}
    >
      <div className="flex max-w-[86%] flex-col md:max-w-[72%]">
        {inGroup && !isOwn && showSender && senderPhone && (
          <span
            className="mb-1 ml-1.5 truncate text-[11px] font-semibold"
            style={senderColor ? { color: senderColor } : undefined}
          >
            {senderPhone}
          </span>
        )}

        <div
          className={cn(
            'relative rounded-2xl shadow-sm',
            !isOwn && 'transition-shadow duration-150 hover:shadow-md',
            hasMedia ? 'p-1.5' : 'px-3.5 py-2',
            isOwn ? 'bubble-out' : 'bubble-in',
            tail && (isOwn ? 'rounded-br-md' : 'rounded-bl-md'),
            message.pending && 'opacity-80',
          )}
        >
          {showCaption && (
            <button
              type="button"
              onClick={(event) => {
                event.stopPropagation();
                onCopyMessage(message);
              }}
              aria-label="Копировать текст"
              title="Копировать текст"
              className={cn(
                'absolute right-1.5 top-1.5 z-10 flex h-7 w-7 items-center justify-center rounded-lg opacity-0 transition-opacity focus:opacity-100 group-hover:opacity-100',
                isOwn
                  ? 'bg-black/25 text-white hover:bg-black/35'
                  : 'bg-panel-strong text-muted hover:text-ink',
              )}
            >
              <Copy className="h-3.5 w-3.5" aria-hidden="true" />
            </button>
          )}

          {hasMedia &&
            (hasImage ? (
              <MediaImage
                api={api}
                message={message}
                isOwn={isOwn}
                onOpen={onOpenImage}
              />
            ) : (
              <FileAttachment
                message={message}
                isOwn={isOwn}
                onDownload={onDownload}
              />
            ))}

          {showCaption && (
            <p
              className={cn(
                'whitespace-pre-wrap break-words text-pretty text-sm leading-relaxed',
                hasMedia && 'px-2 pt-1.5',
                // Отступ под кнопку «Копировать» (не перекрывает текст на ховере).
                showCaption && 'pr-9',
              )}
            >
              {message.text}
            </p>
          )}

          <div
            className={cn(
              'mt-1 flex items-center justify-end gap-1 text-[10px] leading-none',
              hasMedia && 'px-1 pb-0.5',
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
