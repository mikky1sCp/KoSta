import { useState } from 'react';
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

const INITIAL_CLASS: Record<AvatarSize, string> = {
  sm: 'text-sm',
  md: 'text-lg',
  lg: 'text-xl',
  xl: 'text-2xl',
};

function hashString(value: string): number {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 31 + value.charCodeAt(index)) >>> 0;
  }
  return hash;
}

/** Первая буква имени/названия; эмодзи и символы отбрасываются. */
function firstLetter(value: string): string {
  const trimmed = value.trim();
  const first = Array.from(trimmed)[0] ?? '';
  if (first.length > 0 && /^\p{L}$/u.test(first)) {
    return first.toLocaleUpperCase('ru-RU');
  }
  return '';
}

interface AvatarProps {
  /** Название чата или телефон — задаёт цвет фона. */
  title: string;
  isGroup: boolean;
  online?: boolean;
  size?: AvatarSize;
  className?: string;
  /** URL картинки аватара (например, `/api/avatars/7`). */
  src?: string | null;
  /** Имя для инициалов; без него рисуем иконку. */
  initialsFrom?: string | null;
}

/**
 * Аватар чата: картинка, если она есть, иначе инициалы (у групп — первая
 * буква названия, у людей — первая буква имени), иначе пиктограмма.
 */
export function Avatar({
  title,
  isGroup,
  online = false,
  size = 'md',
  className,
  src = null,
  initialsFrom = null,
}: AvatarProps) {
  const [imageFailed, setImageFailed] = useState(false);
  const gradient = GRADIENTS[hashString(title) % GRADIENTS.length];
  const initial = firstLetter(isGroup ? title : initialsFrom ?? '');
  const showImage = Boolean(src) && !imageFailed;

  return (
    <span
      className={cn(
        'relative inline-flex shrink-0 select-none items-center justify-center overflow-hidden border border-black/10 font-semibold text-white shadow-sm',
        SIZE_CLASS[size],
        !showImage && gradient,
        className,
      )}
      aria-hidden="true"
    >
      {showImage ? (
        <img
          src={src ?? undefined}
          alt=""
          className="h-full w-full object-cover"
          loading="lazy"
          onError={() => setImageFailed(true)}
        />
      ) : isGroup ? (
        initial.length > 0 ? (
          <span className={cn('leading-none', INITIAL_CLASS[size])}>{initial}</span>
        ) : (
          <UsersRound className={ICON_CLASS[size]} strokeWidth={2.1} />
        )
      ) : initial.length > 0 ? (
        <span className={cn('leading-none', INITIAL_CLASS[size])}>{initial}</span>
      ) : (
        <UserRound className={ICON_CLASS[size]} strokeWidth={2.1} />
      )}
      {online && (
        <span className="absolute bottom-0.5 right-0.5 h-3 w-3 rounded-full border-2 border-panel bg-online" />
      )}
    </span>
  );
}