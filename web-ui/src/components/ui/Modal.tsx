import { useEffect, useId, type ReactNode } from 'react';
import { X } from 'lucide-react';
import { cn } from '../../utils/classNames';

type ModalSize = 'sm' | 'md' | 'lg';

const SIZE_CLASS: Record<ModalSize, string> = {
  sm: 'sm:max-w-sm',
  md: 'sm:max-w-md',
  lg: 'sm:max-w-lg',
};

interface ModalProps {
  title: string;
  subtitle?: string;
  icon?: ReactNode;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  size?: ModalSize;
}

/** Модальное окно: на мобильном — bottom sheet, на десктопе — центр. */
export function Modal({
  title,
  subtitle,
  icon,
  onClose,
  children,
  footer,
  size = 'md',
}: ModalProps) {
  const titleId = useId();

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
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

  return (
    <div
      className="fixed inset-0 z-50 flex items-end justify-center bg-[var(--overlay)] sm:items-center sm:p-4"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) {
          onClose();
        }
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        className={cn(
          'animate-fade-in-up flex max-h-[92dvh] w-full flex-col overflow-hidden rounded-t-3xl border border-line bg-panel shadow-xl sm:rounded-3xl',
          SIZE_CLASS[size],
        )}
      >
        <header className="flex items-start gap-3 border-b border-line px-5 py-4">
          {icon && (
            <span className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-accent-soft text-accent-strong">
              {icon}
            </span>
          )}
          <div className="min-w-0 flex-1">
            <h2
              id={titleId}
              className="truncate text-base font-semibold tracking-tight text-ink"
            >
              {title}
            </h2>
            {subtitle && (
              <p className="mt-0.5 truncate text-xs text-muted">{subtitle}</p>
            )}
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="Закрыть"
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted transition-colors hover:bg-panel-soft hover:text-ink"
          >
            <X className="h-4 w-4" aria-hidden="true" />
          </button>
        </header>

        <div className="kosta-scroll min-h-0 flex-1 overflow-y-auto px-5 py-4">
          {children}
        </div>

        {footer && <footer className="border-t border-line px-5 py-4">{footer}</footer>}
      </div>
    </div>
  );
}
