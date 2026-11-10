import type { Api } from '../../api';
import type { UiMessage } from '../../types';
import { useMediaUrl } from '../../hooks/useMediaUrl';
import { Spinner } from '../ui/Spinner';

interface AudioAttachmentProps {
  api: Api;
  message: UiMessage;
  isOwn: boolean;
}

/**
 * Голосовое сообщение: файл скачивается авторизованно и играется из blob-url.
 * Нативный `<audio>` даёт перемотку, паузу и громкость без своей логики.
 */
export function AudioAttachment({ api, message, isOwn }: AudioAttachmentProps) {
  const { state } = useMediaUrl(api, message.id);

  return (
    <div className="px-1 pb-0.5 pt-0.5">
      {state.status === 'ready' ? (
        <audio
          src={state.url}
          controls
          preload="metadata"
          aria-label="Голосовое сообщение"
          className="h-10 w-[15rem] max-w-full"
        />
      ) : state.status === 'loading' ? (
        <div
          className="flex h-10 w-[15rem] max-w-full items-center gap-2 px-2"
          role="status"
          aria-label="Загружаем голосовое сообщение"
        >
          <Spinner className="h-4 w-4" />
          <span
            className={
              isOwn ? 'text-xs text-white/80' : 'text-xs text-muted'
            }
          >
            Загружаем…
          </span>
        </div>
      ) : (
        <p
          className={
            isOwn
              ? 'px-2 text-xs text-white/85'
              : 'px-2 text-xs text-danger'
          }
        >
          Не удалось загрузить аудио
        </p>
      )}
    </div>
  );
}