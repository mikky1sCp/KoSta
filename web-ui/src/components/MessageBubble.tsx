import { Check, CheckCheck, Clock3, Copy, Pencil, Reply, Trash2 } from 'lucide-react';
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
  /** Имя автора цитируемого сообщения. */
  replyAuthor: string | null;
  tail: boolean;
  inGroup: boolean;
  api: Api;
  onOpenImage: (message: UiMessage, url: string) => void;
  onDownload: (message: UiMessage) => Promise<void>;
  onCopyMessage: (message: UiMessage) => void;
  onReplyMessage: (message: UiMessage) => void;
  onEditMessage: (message: UiMessage) => void;
  onDeleteMessage: (message: UiMessage) => void;
}

export function MessageBubble({
  message,
  isOwn,
  showSender,
  senderPhone,
  senderColor,
  replyAuthor,
  tail,
  inGroup,
  api,
  onOpenImage,
  onDownload,
  onCopyMessage,
  onReplyMessage,
  onEditMessage,
  onDeleteMessage,
}: MessageBubbleProps) {
  const hasMedia = message.is_media === true;
  const hasImage = hasMedia && isImageMimeType(message.media_type);
  const caption = message.text.trim();
  const showCaption = caption.length > 0;
  const hasReply = message.reply_to_id != null;
  // Редактировать можно только текст (у медиа меняется подпись, но это уже
  // другой сценарий), удалять — любое своё сообщение.
  const canEdit = isOwn && showCaption && !hasMedia && !message.pending;
  const canDelete = isOwn && !message.pending;

  const actionClass = cn(
    'flex h-7 w-7 items-center justify-center rounded-lg transition-colors',
    isOwn
      ? 'text-white/80 hover:bg-black/25 hover:text-white'
      : 'text-muted hover:bg-panel-strong hover:text-ink',
  );

  return (
    <div
      className={cn(
        'animate-fade-in-up group flex w-full',
        isOwn ? 'justify-end' : 'justify-start',
      )}
    >
      <div className="relative flex max-w-[86%] flex-col md:max-w-[72%]">
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
          {/* Панель действий: видна при наведении или фокусе с клавиатуры. */}
          <div
            className={cn(
              'absolute -top-3 z-20 flex items-center gap-0.5 rounded-xl border border-line bg-panel px-1 py-0.5 opacity-0 shadow-md transition-opacity focus-within:opacity-100 group-hover:opacity-100',
              isOwn ? 'right-1' : 'left-1',
            )}
          >
            <button
              type="button"
              onClick={() => onReplyMessage(message)}
              aria-label="Ответить"
              title="Ответить"
              className={actionClass}
            >
              <Reply className="h-3.5 w-3.5" aria-hidden="true" />
            </button>
            {showCaption && (
              <button
                type="button"
                onClick={() => onCopyMessage(message)}
                aria-label="Копировать текст"
                title="Копировать текст"
                className={actionClass}
              >
                <Copy className="h-3.5 w-3.5" aria-hidden="true" />
              </button>
            )}
            {canEdit && (
              <button
                type="button"
                onClick={() => onEditMessage(message)}
                aria-label="Редактировать"
                title="Редактировать"
                className={actionClass}
              >
                <Pencil className="h-3.5 w-3.5" aria-hidden="true" />
              </button>
            )}
            {canDelete && (
              <button
                type="button"
                onClick={() => onDeleteMessage(message)}
                aria-label="Удалить"
                title="Удалить"
                className={cn(actionClass, 'hover:text-danger')}
              >
                <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
              </button>
            )}
          </div>

          {hasReply && (
            <div
              className={cn(
                'mb-1.5 rounded-lg border-l-2 px-2 py-1',
                isOwn
                  ? 'border-white/50 bg-black/15'
                  : 'border-accent bg-accent-soft',
                hasMedia && 'mx-0.5 mt-0.5',
              )}
            >
              <p
                className={cn(
                  'truncate text-[11px] font-semibold',
                  isOwn ? 'text-white/90' : 'text-accent-strong',
                )}
              >
                {replyAuthor ?? 'Сообщение'}
              </p>
              <p
                className={cn(
                  'truncate text-[11px]',
                  isOwn ? 'text-white/75' : 'text-muted',
                )}
              >
                {message.reply_to_text?.trim()
                  ? message.reply_to_text
                  : message.reply_to_is_media
                    ? 'Вложение'
                    : 'Сообщение удалено'}
              </p>
            </div>
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
            {message.edited_at != null && (
              <span className="mr-0.5 italic opacity-90">изменено</span>
            )}
            <span className="tabular-nums">{formatTime(message.timestamp)}</span>
            {isOwn && <OutgoingStatus message={message} />}
          </div>
        </div>
      </div>
    </div>
  );
}
