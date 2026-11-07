import { Loader2 } from 'lucide-react';
import { cn } from '../../utils/classNames';

interface SpinnerProps {
  className?: string;
  label?: string;
}

export function Spinner({ className, label = 'Загрузка' }: SpinnerProps) {
  return (
    <span role="status" aria-label={label} className="inline-flex items-center">
      <Loader2 className={cn('h-4 w-4 animate-spin', className)} aria-hidden="true" />
    </span>
  );
}
