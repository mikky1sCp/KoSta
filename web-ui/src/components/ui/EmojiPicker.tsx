import { useEffect, useRef } from 'react';

/** Часто используемые эмодзи: без внешних зависимостей и тяжёлых баз. */
const EMOJI_GROUPS: { title: string; items: string[] }[] = [
  {
    title: 'Смайлы',
    items: [
      '😀', '😃', '😄', '😁', '😆', '😅', '😂', '🤣',
      '🙂', '🙃', '😉', '😊', '😇', '🥰', '😍', '🤩',
      '😘', '😗', '😚', '😙', '😋', '😛', '😜', '🤪',
      '🤨', '🧐', '🤓', '😎', '🥳', '😏', '😒', '😞',
      '😔', '😟', '😕', '🙁', '😣', '😖', '😫', '😩',
      '🥺', '😢', '😭', '😤', '😠', '😡', '🤬', '🤯',
      '😳', '🥵', '🥶', '😱', '😨', '😰', '😥', '😓',
      '🤗', '🤔', '🤭', '🤫', '🤥', '😶', '😐', '😑',
    ],
  },
  {
    title: 'Жесты и люди',
    items: [
      '👋', '🤚', '🖐', '✋', '🖖', '👌', '🤌', '🤏',
      '✌️', '🤞', '🤟', '🤘', '🤙', '👈', '👉', '👆',
      '👇', '☝️', '👍', '👎', '✊', '👊', '🤛', '🤜',
      '👏', '🙌', '👐', '🤲', '🤝', '🙏', '💪', '🫶',
      '❤️', '🧡', '💛', '💚', '💙', '💜', '🖤', '🤍',
      '💔', '❣️', '💕', '💞', '💓', '💗', '💖', '💘',
    ],
  },
  {
    title: 'Животные и природа',
    items: [
      '🐶', '🐱', '🐭', '🐹', '🐰', '🦊', '🐻', '🐼',
      '🐨', '🐯', '🦁', '🐮', '🐷', '🐸', '🐵', '🙈',
      '🐔', '🐧', '🐦', '🦆', '🦉', '🦄', '🐝', '🦋',
      '🌵', '🌲', '🌳', '🌴', '🌱', '🌿', '🍀', '🍁',
      '🌸', '🌼', '🌻', '🌞', '🌝', '🌚', '🌙', '⭐',
      '🔥', '💧', '❄️', '⚡', '🌈', '☀️', '⛅', '🌊',
    ],
  },
  {
    title: 'Еда и вещи',
    items: [
      '🍏', '🍎', '🍐', '🍊', '🍋', '🍌', '🍉', '🍇',
      '🍓', '🫐', '🍒', '🍑', '🥭', '🍍', '🥥', '🥝',
      '🍅', '🥑', '🥦', '🌽', '🥕', '🍞', '🧀', '🍕',
      '🍔', '🍟', '🌮', '🍣', '🍜', '🍩', '🍪', '🎂',
      '☕', '🍵', '🥤', '🍺', '🍷', '🥂', '🎁', '🎉',
      '🎈', '💡', '📱', '💻', '⌨️', '🎧', '📷', '🚀',
    ],
  },
];

interface EmojiPickerProps {
  onPick: (emoji: string) => void;
  onClose: () => void;
}

/** Компактная панель эмодзи: закрывается по клику вне и по Escape. */
export function EmojiPicker({ onPick, onClose }: EmojiPickerProps) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handlePointerDown = (event: MouseEvent | TouchEvent) => {
      if (
        containerRef.current &&
        !containerRef.current.contains(event.target as Node)
      ) {
        onClose();
      }
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        onClose();
      }
    };
    document.addEventListener('mousedown', handlePointerDown);
    document.addEventListener('touchstart', handlePointerDown);
    document.addEventListener('keydown', handleKeyDown);
    return () => {
      document.removeEventListener('mousedown', handlePointerDown);
      document.removeEventListener('touchstart', handlePointerDown);
      document.removeEventListener('keydown', handleKeyDown);
    };
  }, [onClose]);

  return (
    <div
      ref={containerRef}
      role="dialog"
      aria-label="Выбор эмодзи"
      className="animate-fade-in-up absolute bottom-14 left-0 z-30 w-[min(21rem,calc(100vw-2rem))] rounded-2xl border border-line bg-panel p-2 shadow-xl"
    >
      <div className="kosta-scroll max-h-64 overflow-y-auto">
        {EMOJI_GROUPS.map((group) => (
          <div key={group.title} className="mb-1">
            <p className="px-1 pb-1 pt-1.5 text-[10px] font-semibold uppercase tracking-wide text-muted">
              {group.title}
            </p>
            <div className="grid grid-cols-8 gap-0.5">
              {group.items.map((emoji) => (
                <button
                  key={`${group.title}-${emoji}`}
                  type="button"
                  onClick={() => onPick(emoji)}
                  className="flex h-8 w-8 items-center justify-center rounded-lg text-lg leading-none transition-colors hover:bg-panel-strong"
                  aria-label={emoji}
                >
                  {emoji}
                </button>
              ))}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}