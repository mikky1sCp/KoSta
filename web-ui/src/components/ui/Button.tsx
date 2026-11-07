import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { Loader2 } from 'lucide-react';
import { cn } from '../../utils/classNames';

type ButtonVariant = 'primary' | 'secondary' | 'ghost' | 'danger';
type ButtonSize = 'sm' | 'md';

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  icon?: ReactNode;
  fullWidth?: boolean;
}

const VARIANT_CLASS: Record<ButtonVariant, string> = {
  primary:
    'bg-gradient-to-br from-indigo-500 to-violet-600 text-white shadow-lg shadow-indigo-900/30 hover:from-indigo-400 hover:to-violet-500 disabled:from-indigo-500/60 disabled:to-violet-600/60',
  secondary:
    'border border-line bg-panel-soft text-ink hover:bg-panel-strong disabled:opacity-60',
  ghost: 'text-muted hover:bg-panel-soft hover:text-ink disabled:opacity-60',
  danger:
    'border border-danger/30 bg-danger/10 text-danger hover:bg-danger/15 disabled:opacity-60',
};

const SIZE_CLASS: Record<ButtonSize, string> = {
  sm: 'h-9 gap-1.5 rounded-lg px-3 text-xs font-medium',
  md: 'h-11 gap-2 rounded-xl px-4 text-sm font-semibold',
};

export function Button({
  variant = 'primary',
  size = 'md',
  loading = false,
  icon,
  fullWidth = false,
  className,
  children,
  disabled,
  type = 'button',
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={cn(
        'inline-flex items-center justify-center whitespace-nowrap transition-colors disabled:cursor-not-allowed',
        VARIANT_CLASS[variant],
        SIZE_CLASS[size],
        fullWidth && 'w-full',
        className,
      )}
      disabled={disabled || loading}
      {...rest}
    >
      {loading ? (
        <Loader2 className="h-4 w-4 animate-spin" aria-hidden="true" />
      ) : (
        icon
      )}
      {children}
    </button>
  );
}
