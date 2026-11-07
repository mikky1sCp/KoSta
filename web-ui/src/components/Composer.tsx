import {
  useEffect,
  useRef,
  useState,
  type ChangeEvent,
  type KeyboardEvent,
} from 'react';
import { Paperclip, Send, WifiOff, X } from 'lucide-react';
import { MEDIA_ACCEPT } from '../api';
import { formatBytes } from '../utils/format';
import {
  isImageFile,
  mediaTypeLabel,
  validateMediaFile,
} from '../utils/media';
import { cn } from '../utils/classNames';
import { FileKindIcon } from './media/FileKindIcon';

const MAX_MESSAGE_LENGTH = 4096;

interface ComposerProps {
  chatId: number;
  connected: boolean;
  onSend: (text: string) => boolean;
  /** Загрузка вложения: true — сообщение ушло в чат. */
  onUpload: (
    file: File,
    caption: string,
    onProgress: (percent: number) => void,
  ) => Promise<boolean>;
  onTypingInput: (hasText: boolean) => void;
}

export function Composer({
  chatId,
  connected,
  onSend,
  onUpload,
  onTypingInput,
}: ComposerProps) {
  const [text, setText] = useState('');
  const [pendingFile, setPendingFile] = useState<File | null>(null);
  const [fileError, setFileError] = useState('');
  const [uploading, setUploading] = useState(false);
  const [progress, setProgress] = useState(0);
  const [previewUrl, setPreviewUrl] = useState<string | null>(null);

  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);

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

  // Локальная миниатюра выбранного изображения (файл ещё не загружен).
  useEffect(() => {
    if (!pendingFile || !isImageFile(pendingFile)) {
      setPreviewUrl(null);
      return;
    }
    const url = URL.createObjectURL(pendingFile);
    setPreviewUrl(url);
    return () => URL.revokeObjectURL(url);
  }, [pendingFile]);

  const trimmed = text.trim();
  const canSend = !uploading && (pendingFile !== null || trimmed.length > 0);

  function clearFile() {
    setPendingFile(null);
    setFileError('');
    setProgress(0);
  }

  function handleFileChange(event: ChangeEvent<HTMLInputElement>) {
    const input = event.currentTarget;
    const file = input.files?.[0] ?? null;
    // Сбрасываем value, чтобы можно было выбрать тот же файл повторно.
    input.value = '';
    if (!file) {
      return;
    }
    const error = validateMediaFile(file);
    if (error) {
      setFileError(error);
      return;
    }
    setFileError('');
    setPendingFile(file);
    textareaRef.current?.focus();
  }

  async function uploadPending() {
    const file = pendingFile;
    if (!file || uploading) {
      return;
    }
    const caption = trimmed;
    setFileError('');
    setProgress(0);
    setUploading(true);
    let uploaded = false;
    try {
      uploaded = await onUpload(file, caption, (percent) => setProgress(percent));
    } catch {
      uploaded = false;
    } finally {
      setUploading(false);
    }

    if (!uploaded) {
      setProgress(0);
      setFileError('Не удалось отправить файл. Попробуйте ещё раз.');
      return;
    }

    setPendingFile(null);
    setProgress(0);
    // Текст, набранный уже во время загрузки, не затираем.
    setText((current) => (current.trim() === caption ? '' : current));
    onTypingInput(false);
  }

  async function handleSend() {
    if (uploading) {
      return;
    }
    if (pendingFile) {
      await uploadPending();
      return;
    }
    if (!trimmed) {
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
      void handleSend();
    }
  }

  function handleChange(value: string) {
    setText(value);
    onTypingInput(value.trim().length > 0);
  }

  const sendLabel = pendingFile ? 'Отправить файл' : 'Отправить сообщение';

  return (
    <div className="border-t border-line bg-panel/70 px-3 py-3 backdrop-blur md:px-5">
      <div className="mx-auto w-full max-w-3xl">
        {!connected && (
          <div className="mb-2 flex items-center gap-2 rounded-xl border border-warn/25 bg-warn/10 px-3 py-1.5 text-xs text-warn">
            <WifiOff className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
            Соединение потеряно. Переподключаемся…
          </div>
        )}

        {pendingFile && (
          <div className="mb-2 flex items-center gap-3 rounded-xl border border-line bg-panel-soft px-2.5 py-2">
            {previewUrl ? (
              <img
                src={previewUrl}
                alt=""
                className="h-11 w-11 shrink-0 rounded-lg object-cover"
              />
            ) : (
              <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg bg-panel-strong text-accent-strong">
                <FileKindIcon mime={pendingFile.type} className="h-5 w-5" />
              </span>
            )}

            <div className="min-w-0 flex-1">
              <p
                className="truncate text-xs font-medium text-ink"
                title={pendingFile.name}
              >
                {pendingFile.name}
              </p>
              <p className="mt-0.5 text-[11px] text-muted">
                {mediaTypeLabel(pendingFile.type)} · {formatBytes(pendingFile.size)}
                {uploading ? ` · ${progress}%` : ''}
              </p>
              {uploading && (
                <div
                  className="mt-1.5 h-1 overflow-hidden rounded-full bg-panel-strong"
                  role="progressbar"
                  aria-label="Загрузка файла"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={progress}
                >
                  <div
                    className="h-full rounded-full bg-gradient-to-r from-indigo-500 to-violet-500 transition-[width] duration-150 ease-out"
                    style={{ width: `${progress}%` }}
                  />
                </div>
              )}
            </div>

            <button
              type="button"
              onClick={clearFile}
              disabled={uploading}
              aria-label="Убрать файл"
              title="Убрать файл"
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-muted transition-colors hover:bg-panel-strong hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
            >
              <X className="h-4 w-4" aria-hidden="true" />
            </button>
          </div>
        )}

        {fileError && (
          <p
            role="alert"
            className="mb-2 rounded-xl border border-danger/25 bg-danger/10 px-3 py-1.5 text-xs text-danger"
          >
            {fileError}
          </p>
        )}

        <div className="flex items-end gap-2 rounded-2xl border border-line bg-panel-soft p-2 transition-colors focus-within:border-accent/60">
          <input
            ref={fileInputRef}
            type="file"
            accept={MEDIA_ACCEPT}
            onChange={handleFileChange}
            disabled={uploading}
            tabIndex={-1}
            aria-hidden="true"
            className="hidden"
          />

          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            disabled={uploading}
            aria-label="Прикрепить файл"
            title="Прикрепить файл"
            className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl text-muted transition-colors hover:bg-panel-strong hover:text-ink disabled:cursor-not-allowed disabled:opacity-50"
          >
            <Paperclip className="h-4 w-4" aria-hidden="true" />
          </button>

          <textarea
            ref={textareaRef}
            value={text}
            onChange={(event) => handleChange(event.target.value)}
            onKeyDown={handleKeyDown}
            maxLength={MAX_MESSAGE_LENGTH}
            rows={1}
            placeholder={
              pendingFile
                ? 'Добавить подпись…'
                : connected
                  ? 'Сообщение…'
                  : 'Ожидание соединения…'
            }
            aria-label={pendingFile ? 'Подпись к файлу' : 'Текст сообщения'}
            className="kosta-scroll max-h-40 min-h-10 w-full resize-none bg-transparent px-2 py-2 text-sm leading-relaxed text-ink outline-none placeholder:text-muted"
          />

          <button
            type="button"
            onClick={() => void handleSend()}
            disabled={!canSend}
            aria-label={sendLabel}
            title={sendLabel}
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
          {uploading && (
            <span className="tabular-nums" role="status">
              Отправляем файл…
            </span>
          )}
          {!uploading && text.length > 0 && (
            <span className={cn('tabular-nums', text.length > 3900 && 'text-warn')}>
              {text.length} / {MAX_MESSAGE_LENGTH}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}
