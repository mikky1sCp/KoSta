import { useState } from 'react';
import { Download, Loader2 } from 'lucide-react';
import type { UiMessage } from '../../types';
import { formatBytes } from '../../utils/format';
import { mediaTypeLabel } from '../../utils/media';
import { cn } from '../../utils/classNames';
import { FileKindIcon } from './FileKindIcon';

interface FileAttachmentProps {
  message: UiMessage;
  isOwn: boolean;
  onDownload: (message: UiMessage) => Promise<void>;
}

/** Карточка не-изображения: иконка по MIME, имя, размер, скачивание. */
export function FileAttachment({ message, isOwn, onDownload }: FileAttachmentProps) {
  const [downloading, setDownloading] = useState(false);
  const name = message.media_name?.trim() || 'Файл';

  async function handleDownload() {
    if (downloading) {
      return;
    }
    setDownloading(true);
    try {
      await onDownload(message);
    } finally {
      setDownloading(false);
    }
  }

  return (
    <div
      className={cn(
        'flex w-60 max-w-full items-center gap-2.5 rounded-xl p-2 sm:w-64',
        isOwn ? 'bg-white/15' : 'border border-line bg-panel-soft',
      )}
    >
      <span
        className={cn(
          'flex h-9 w-9 shrink-0 items-center justify-center rounded-lg',
          isOwn ? 'bg-white/20 text-white' : 'bg-accent-soft text-accent-strong',
        )}
      >
        <FileKindIcon mime={message.media_type} className="h-[18px] w-[18px]" />
      </span>

      <span className="min-w-0 flex-1">
        <span
          className={cn(
            'block truncate text-xs font-medium',
            isOwn ? 'text-white' : 'text-ink',
          )}
          title={name}
        >
          {name}
        </span>
        <span
          className={cn('mt-0.5 block text-[11px]', isOwn ? 'text-white/75' : 'text-muted')}
        >
          {mediaTypeLabel(message.media_type)} · {formatBytes(message.media_size)}
        </span>
      </span>

      <button
        type="button"
        onClick={() => void handleDownload()}
        disabled={downloading}
        aria-label={`Скачать ${name}`}
        title="Скачать"
        className={cn(
          'flex h-8 w-8 shrink-0 items-center justify-center rounded-lg transition-colors disabled:opacity-60',
          isOwn
            ? 'text-white/90 hover:bg-white/20 hover:text-white'
            : 'text-muted hover:bg-panel-strong hover:text-ink',
        )}
      >
        {downloading ? (
          <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
        ) : (
          <Download className="h-4 w-4" aria-hidden="true" />
        )}
      </button>
    </div>
  );
}
