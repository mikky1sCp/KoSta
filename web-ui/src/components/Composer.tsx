import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { Send, WifiOff } from 'lucide-react';
import { cn } from '../utils/classNames';

const MAX_MESSAGE_LENGTH = 4096;

interface ComposerProps {
  chatId: number;
  connected: boolean;
  onSend: (text: string) => boolean;
  onTypingInput: (hasText: boolean) => void;
}

export function Composer({
  chatId,
  connected,
  onSend,
  onTypingInput,
}: ComposerProps) {
  const [text, setText] = useState('');
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  // Textarea растёт по контенту до 10 строк.
  useEffect(() => {
    const element = textareaRef.current;
    if (!element) {
      return;
    }
    element.style.height = 'auto';
    element.style.height = `${Math.min(element.scrollHeight, 160)}px`;
  }, [text]);

  useEffect(() => {
    textareaRef.current?.focus();
  }, [chatId]);

  const trimmed = text.trim();
  const canSend = trimmed.length > 0;

  function handleSend() {
    if (!canSend) {
      return;
    }
    const sent = onSend(trimmed);
    if (!sent) {
      return;
    }
    setText('');
    onTypingInput(false);
  }

  function handleKeyDown(event: KeyboardEvent<HTMLTextAreaElement>) {
    if (event.key === 'Enter' && !event.shiftKey) {
      if (event.nativeEvent.isComposing) {
        return;
      }
      event.preventDefault();
      handleSend();
    }
  }

  function handleChange(value: string) {
    setText(value);
    onTypingInput(value.trim().length > 0);
  }

  return (
    <div className="border-t border-line bg-panel/70 px-3 py-3 backdrop-blur md:px-5">
      <div className="mx-auto w-full max-w-3xl">
        {!connected && (
          <div className="mb-2 flex items-center gap-2 rounded-xl border border-warn/25 bg-warn/10 px-3 py-1.5 text-xs text-warn">
            <WifiOff className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
            Соединение потеряно. Переподключаемся…
          </div>
        )}

        <div className="flex items-end gap-2 rounded-2xl border border-line bg-panel-soft p-2 transition-colors focus-within:border-accent/60">
          <textarea
            ref={textareaRef}
            value={text}
            onChange={(event) => handleChange(event.target.value)}
            onKeyDown={handleKeyDown}
            maxLength={MAX_MESSAGE_LENGTH}
            rows={1}
            placeholder={connected ? 'Сообщение…' : 'Ожидание соединения…'}
            aria-label="Текст сообщения"
            className="kosta-scroll max-h-40 min-h-10 w-full resize-none bg-transparent px-2 py-2 text-sm leading-relaxed text-ink outline-none placeholder:text-muted"
          />
          <button
            type="button"
            onClick={handleSend}
            disabled={!canSend}
            aria-label="Отправить сообщение"
            className={cn(
              'flex h-10 w-10 shrink-0 items-center justify-center rounded-xl transition-all',
              canSend
                ? 'bg-gradient-to-br from-indigo-500 to-violet-600 text-white shadow-lg shadow-indigo-900/30 hover:from-indigo-400 hover:to-violet-500 active:scale-95'
                : 'cursor-not-allowed bg-panel-strong text-muted',
            )}
          >
            <Send className="h-4 w-4" aria-hidden="true" />
          </button>
        </div>

        <div className="mt-1 flex items-center justify-between gap-2 px-1 text-[11px] text-muted">
          <span className="hidden sm:inline">
            Enter — отправить · Shift+Enter — новая строка
          </span>
          {text.length > 0 && (
            <span className={cn('tabular-nums', text.length > 3900 && 'text-warn')}>
              {text.length} / {MAX_MESSAGE_LENGTH}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
