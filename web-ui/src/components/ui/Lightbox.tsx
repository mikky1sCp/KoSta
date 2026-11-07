import { useEffect, useRef, useState, type MouseEvent } from 'react';
import { Download, Loader2, X } from 'lucide-react';

interface LightboxProps {
  /** Blob-url уже авторизованно скачанного файла. */
  src: string;
  /** Имя файла — заголовок и alt. */
  title: string;
  caption?: string;
  onClose: () => void;
  /** Скачивание исходного файла (тем же blob-url не пользуемся). */
  onDownload?: () => Promise<void> | void;
}

/**
 * Полноэкранный просмотр изображения без сторонних библиотек:
 * закрытие по Esc и клику по фону, фокус на кнопке закрытия.
 */
export function Lightbox({ src, title, caption, onClose, onDownload }: LightboxProps) {
  const closeButtonRef = useRef<HTMLButtonElement>(null);
  const [downloading, setDownloading] = useState(false);

  useEffect(() => {
    closeButtonRef.current?.focus();
  }, []);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        onClose();
      }
    };
    document.addEventListener('keydown', handleKeyDown);
    const previousOverflow = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      document.removeEventListener('keydown', handleKeyDown);
      document.body.style.overflow = previousOverflow;
    };
  }, [onClose]);

  function handleBackdropMouseDown(event: MouseEvent<HTMLDivElement>) {
    if (event.target === event.currentTarget) {
      onClose();
    }
  }

  async function handleDownload() {
    if (!onDownload || downloading) {
      return;
    }
    setDownloading(true);
    try {
      await onDownload();
    } finally {
      setDownloading(false);
    }
  }

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={title}
      className="fixed inset-0 z-[60] flex flex-col bg-black/90 backdrop-blur-sm"
      onMouseDown={handleBackdropMouseDown}
    >
      <div className="flex items-center gap-2 px-4 py-3 text-white">
        <p className="min-w-0 flex-1 truncate text-sm font-medium" title={title}>
          {title}
        </p>
        {onDownload && (
          <button
            type="button"
            onClick={() => void handleDownload()}
            disabled={downloading}
            aria-label="Скачать файл"
            title="Скачать"
            className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-white/85 transition-colors hover:bg-white/15 hover:text-white disabled:opacity-60"
          >
            {downloading ? (
              <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
            ) : (
              <Download className="h-4 w-4" aria-hidden="true" />
            )}
          </button>
        )}
        <button
          ref={closeButtonRef}
          type="button"
          onClick={onClose}
          aria-label="Закрыть просмотр"
          title="Закрыть (Esc)"
          className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl text-white/85 transition-colors hover:bg-white/15 hover:text-white"
        >
          <X className="h-4 w-4" aria-hidden="true" />
        </button>
      </div>

      <div
        className="flex min-h-0 flex-1 items-center justify-center px-2 pb-4 sm:px-6"
        onMouseDown={handleBackdropMouseDown}
      >
        <img
          src={src}
          alt={title}
          className="max-h-full max-w-full rounded-lg object-contain shadow-2xl"
        />
      </div>

      {caption && (
        <p className="mx-auto max-w-3xl px-4 pb-5 text-center text-sm leading-relaxed text-white/85">
          {caption}
        </p>
      )}
    </div>
  );
}
