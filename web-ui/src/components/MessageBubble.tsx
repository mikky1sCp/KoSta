import { useEffect, useRef, useState } from 'react';
import {
  Check,
  CheckCheck,
  Clock3,
  Copy,
  MoreHorizontal,
  Pencil,
  Reply,
  Trash2,
} from 'lucide-react';
import type { Api } from '../api';
import type { UiMessage } from '../types';
import { formatTime } from '../utils/format';
import { isAudioMimeType, isImageMimeType } from '../utils/media';
import { cn } from '../utils/classNames';
import { AudioAttachment } from './media/AudioAttachment';
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
  const hasAudio = hasMedia && isAudioMimeType(message.media_type);
  const caption = message.text.trim();
  const showCaption = caption.length > 0;
  const hasReply = message.reply_to_id != null;
  // Редактировать можно только текст (у медиа меняется подпись, но это уже
  // другой сценарий), удалять — любое своё сообщение.
  const canEdit = isOwn && showCaption && !hasMedia && !message.pending;
  const canDelete = isOwn && !message.pending;

  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement>(null);

  // Меню закрывается по клику вне его и по Escape.
  useEffect(() => {
    if (!menuOpen) {
      return;
    }
    const handlePointerDown = (event: MouseEvent | TouchEvent) => {
      if (menuRef.current && !menuRef.current.contains(event.target as Node)) {
        setMenuOpen(false);
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setMenuOpen(false);
      }
    };
    document.addEventListener('mousedown', handlePointerDown);
    document.addEventListener('touchstart', handlePointerDown);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
      document.removeEventListener('touchstart', handlePointerDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [menuOpen]);

  function runAction(action: (message: UiMessage) => void) {
    setMenuOpen(false);
    action(message);
  }

  const menuItemClass =
    'flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs text-ink transition-colors hover:bg-panel-strong';

  return (
    <div
      className={cn(
        'animate-fade-in-up flex w-full',
        // Анимация создаёт собственный контекст наложения у каждого сообщения,
        // поэтому открытое меню иначе перекрывается следующим сообщением.
        menuOpen && 'relative z-40',
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

        <div className="relative flex items-start gap-1">
          {/* Кнопка «⋯» видна всегда: hover-меню не работает на тач-экранах
              и его трудно обнаружить. */}
          <div ref={menuRef} className={cn('relative shrink-0', isOwn && 'order-2')}>
            <button
              type="button"
              onClick={() => setMenuOpen((open) => !open)}
              aria-label="Действия с сообщением"
              aria-expanded={menuOpen}
              title="Действия"
              className="flex h-6 w-6 items-center justify-center rounded-full text-muted opacity-45 transition-opacity hover:bg-panel-soft hover:text-ink hover:opacity-100 focus-visible:opacity-100"
            >
              <MoreHorizontal className="h-4 w-4" aria-hidden="true" />
            </button>

            {menuOpen && (
              <div
                role="menu"
                className={cn(
                  'absolute top-7 z-30 w-40 rounded-xl border border-line bg-panel p-1 shadow-xl',
                  isOwn ? 'right-0' : 'left-0',
                )}
              >
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => runAction(onReplyMessage)}
                  className={menuItemClass}
                >
                  <Reply className="h-3.5 w-3.5" aria-hidden="true" />
                  Ответить
                </button>
                {showCaption && (
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => runAction(onCopyMessage)}
                    className={menuItemClass}
                  >
                    <Copy className="h-3.5 w-3.5" aria-hidden="true" />
                    Копировать
                  </button>
                )}
                {canEdit && (
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => runAction(onEditMessage)}
                    className={menuItemClass}
                  >
                    <Pencil className="h-3.5 w-3.5" aria-hidden="true" />
                    Редактировать
                  </button>
                )}
                {canDelete && (
                  <button
                    type="button"
                    role="menuitem"
                    onClick={() => runAction(onDeleteMessage)}
                    className={cn(menuItemClass, 'text-danger hover:bg-danger/10')}
                  >
                    <Trash2 className="h-3.5 w-3.5" aria-hidden="true" />
                    Удалить
                  </button>
                )}
              </div>
            )}
          </div>

          <div
            className={cn(
              'min-w-0 rounded-2xl shadow-sm',
              hasMedia ? 'p-1.5' : 'px-3.5 py-2',
              isOwn ? 'bubble-out' : 'bubble-in',
              tail && (isOwn ? 'rounded-br-md' : 'rounded-bl-md'),
              message.pending && 'opacity-80',
            )}
          >
            {hasReply && (
              <div
                className={cn(
                  'mb-1.5 rounded-lg border-l-2 px-2 py-1',
                  isOwn ? 'border-white/50 bg-black/15' : 'border-accent bg-accent-soft',
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
              ) : hasAudio ? (
                <AudioAttachment api={api} message={message} isOwn={isOwn} />
              ) : (
                <FileAttachment
                  message={message}
                  isOwn={isOwn}
                  onDownload={onDownload}
                />
              ))}

            {showCaption && (
              <p className="whitespace-pre-wrap break-words text-pretty text-sm leading-relaxed">
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
    </div>
  );
}
