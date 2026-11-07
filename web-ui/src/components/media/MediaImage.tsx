import { FileImage, RefreshCw, ZoomIn } from 'lucide-react';
import type { Api } from '../../api';
import type { UiMessage } from '../../types';
import { useMediaUrl } from '../../hooks/useMediaUrl';
import { Spinner } from '../ui/Spinner';
import { cn } from '../../utils/classNames';

interface MediaImageProps {
  api: Api;
  message: UiMessage;
  isOwn: boolean;
  /** Клик по превью: открывает лайтбокс с уже загруженным blob-url. */
  onOpen: (message: UiMessage, url: string) => void;
}

/** Превью изображения: скелетон → превью или плейсхолдер с повтором. */
export function MediaImage({ api, message, isOwn, onOpen }: MediaImageProps) {
  const { state, reload } = useMediaUrl(api, message.id);
  const alt = message.media_name?.trim() || 'Изображение';

  if (state.status === 'loading') {
    return (
      <div
        aria-busy="true"
        aria-label="Загрузка изображения"
        className={cn(
          'flex h-40 w-56 items-center justify-center rounded-xl',
          isOwn ? 'bg-white/15 text-white/80' : 'bg-panel-strong text-muted',
        )}
      >
        <Spinner label="Загрузка изображения" />
      </div>
    );
  }

  if (state.status === 'error') {
    return (
      <div
        role="group"
        aria-label={`Не удалось загрузить изображение ${alt}`}
        className={cn(
          'flex h-32 w-52 flex-col items-center justify-center gap-2 rounded-xl px-3 text-center',
          isOwn ? 'bg-white/15 text-white/85' : 'bg-panel-strong text-muted',
        )}
      >
        <FileImage className="h-5 w-5" aria-hidden="true" />
        <p className="text-[11px] leading-snug">
          Не удалось загрузить изображение
        </p>
        <button
          type="button"
          onClick={reload}
          className={cn(
            'flex items-center gap-1 rounded-lg px-2 py-1 text-[11px] font-medium transition-colors',
            isOwn
              ? 'text-white hover:bg-white/20'
              : 'text-accent-strong hover:bg-panel',
          )}
        >
          <RefreshCw className="h-3 w-3" aria-hidden="true" />
          Повторить
        </button>
        <span className="sr-only">{state.message}</span>
      </div>
    );
  }

  return (
    <button
      type="button"
      onClick={() => onOpen(message, state.url)}
      aria-label={`Открыть изображение ${alt}`}
      className="group relative block w-fit max-w-[280px] overflow-hidden rounded-xl"
    >
      <img
        src={state.url}
        alt={alt}
        loading="lazy"
        className="block h-auto max-h-[260px] w-auto max-w-full object-contain"
      />
      <span
        aria-hidden="true"
        className="absolute inset-0 flex items-center justify-center bg-black/0 opacity-0 transition-all group-hover:bg-black/25 group-hover:opacity-100"
      >
        <ZoomIn className="h-5 w-5 text-white drop-shadow" />
      </span>
    </button>
  );
}
