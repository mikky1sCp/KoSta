import { AlertCircle, CheckCircle2, Info, X } from 'lucide-react';
import type { ToastItem } from '../../hooks/useToasts';
import type { ToastKind } from '../../types';
import { cn } from '../../utils/classNames';

interface ToastVariant {
  icon: typeof AlertCircle;
  className: string;
  iconClassName: string;
}

const VARIANTS: Record<ToastKind, ToastVariant> = {
  error: {
    icon: AlertCircle,
    className: 'border-danger/30 bg-panel text-ink',
    iconClassName: 'text-danger',
  },
  success: {
    icon: CheckCircle2,
    className: 'border-online/30 bg-panel text-ink',
    iconClassName: 'text-online',
  },
  info: {
    icon: Info,
    className: 'border-accent/30 bg-panel text-ink',
    iconClassName: 'text-accent-strong',
  },
};

interface ToastViewportProps {
  toasts: ToastItem[];
  onDismiss: (id: number) => void;
}

export function ToastViewport({ toasts, onDismiss }: ToastViewportProps) {
  if (toasts.length === 0) {
    return null;
  }

  return (
    <div className="pointer-events-none fixed inset-x-0 bottom-4 z-[60] flex flex-col items-center gap-2 px-4 sm:bottom-6">
      {toasts.map((toast) => {
        const variant = VARIANTS[toast.kind];
        const Icon = variant.icon;
        return (
          <div
            key={toast.id}
            role="status"
            aria-live="polite"
            className={cn(
              'animate-toast-in pointer-events-auto flex w-full max-w-md items-start gap-2.5 rounded-2xl border px-3.5 py-3 shadow-lg',
              variant.className,
            )}
          >
            <Icon
              className={cn('mt-0.5 h-4 w-4 shrink-0', variant.iconClassName)}
              aria-hidden="true"
            />
            <p className="min-w-0 flex-1 break-words text-sm leading-snug">
              {toast.text}
            </p>
            <button
              type="button"
              onClick={() => onDismiss(toast.id)}
              aria-label="Скрыть уведомление"
              className="flex h-6 w-6 shrink-0 items-center justify-center rounded-md text-muted transition-colors hover:bg-panel-soft hover:text-ink"
            >
              <X className="h-3.5 w-3.5" aria-hidden="true" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
