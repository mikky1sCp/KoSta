import { UsersRound, UserRound } from 'lucide-react';
import { cn } from '../../utils/classNames';

const GRADIENTS = [
  'avatar-grad-0',
  'avatar-grad-1',
  'avatar-grad-2',
  'avatar-grad-3',
  'avatar-grad-4',
] as const;

type AvatarSize = 'sm' | 'md' | 'lg' | 'xl';

const SIZE_CLASS: Record<AvatarSize, string> = {
  sm: 'h-9 w-9 rounded-xl',
  md: 'h-11 w-11 rounded-2xl',
  lg: 'h-14 w-14 rounded-2xl',
  xl: 'h-20 w-20 rounded-3xl',
};

const ICON_CLASS: Record<AvatarSize, string> = {
  sm: 'h-4 w-4',
  md: 'h-5 w-5',
  lg: 'h-6 w-6',
  xl: 'h-8 w-8',
};

function hashString(value: string): number {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 31 + value.charCodeAt(index)) >>> 0;
  }
  return hash;
}

interface AvatarProps {
  title: string;
  isGroup: boolean;
  online?: boolean;
  size?: AvatarSize;
  className?: string;
}

/** Аватар чата: градиент выбирается детерминированно по названию. */
export function Avatar({
  title,
  isGroup,
  online = false,
  size = 'md',
  className,
}: AvatarProps) {
  const gradient = GRADIENTS[hashString(title) % GRADIENTS.length];
  return (
    <span
      className={cn(
        'relative inline-flex shrink-0 items-center justify-center text-white shadow-sm',
        SIZE_CLASS[size],
        gradient,
        className,
      )}
      aria-hidden="true"
    >
      {isGroup ? (
        <UsersRound className={ICON_CLASS[size]} strokeWidth={2.1} />
      ) : (
        <UserRound className={ICON_CLASS[size]} strokeWidth={2.1} />
      )}
      {online && (
        <span className="status-dot-online absolute -bottom-0.5 -right-0.5 h-3 w-3 rounded-full border-2 border-panel bg-online" />
      )}
    </span>
  );
}
