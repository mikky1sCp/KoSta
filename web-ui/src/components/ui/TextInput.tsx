import { useId, type InputHTMLAttributes, type ReactNode } from 'react';
import { cn } from '../../utils/classNames';

interface TextInputProps extends InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  icon?: ReactNode;
  trailing?: ReactNode;
  hint?: string;
  containerClassName?: string;
}

/** Текстовое поле с иконками и подписью в общей стилистике. */
export function TextInput({
  label,
  icon,
  trailing,
  hint,
  id,
  className,
  containerClassName,
  ...rest
}: TextInputProps) {
  const generatedId = useId();
  const inputId = id ?? generatedId;

  return (
    <div className={cn('block', containerClassName)}>
      {label && (
        <label
          htmlFor={inputId}
          className="mb-1.5 block text-xs font-medium text-muted"
        >
          {label}
        </label>
      )}
      <div className="flex items-center gap-2 rounded-xl border border-line bg-panel-soft px-3 transition focus-within:border-accent/70 focus-within:ring-2 focus-within:ring-accent/15">
        {icon && <span className="text-muted">{icon}</span>}
        <input
          id={inputId}
          className={cn(
            'h-11 w-full bg-transparent text-sm text-ink outline-none placeholder:text-muted disabled:opacity-60',
            className,
          )}
          {...rest}
        />
        {trailing}
      </div>
      {hint && <p className="mt-1 text-[11px] text-muted">{hint}</p>}
    </div>
  );
}
