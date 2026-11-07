import { useCallback, useEffect, useState } from 'react';
import type { Api } from '../api';
import { getErrorMessage } from '../utils/errors';

export type MediaUrlState =
  | { status: 'loading' }
  | { status: 'ready'; url: string }
  | { status: 'error'; message: string };

/**
 * Blob-url для `GET /api/media/{id}`: `<img src>` не отправит Authorization,
 * поэтому файл скачивается через api и превращается в object-url.
 * URL освобождается при размонтировании/смене сообщения.
 */
export function useMediaUrl(
  api: Api,
  messageId: number,
  enabled = true,
): { state: MediaUrlState; reload: () => void } {
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<MediaUrlState>({ status: 'loading' });

  useEffect(() => {
    if (!enabled) {
      return;
    }
    let cancelled = false;
    let objectUrl: string | null = null;
    setState({ status: 'loading' });

    void api
      .fetchMediaBlob(messageId)
      .then((blob) => {
        if (cancelled) {
          return;
        }
        objectUrl = URL.createObjectURL(blob);
        setState({ status: 'ready', url: objectUrl });
      })
      .catch((error: unknown) => {
        if (cancelled) {
          return;
        }
        setState({ status: 'error', message: getErrorMessage(error) });
      });

    return () => {
      cancelled = true;
      if (objectUrl) {
        URL.revokeObjectURL(objectUrl);
      }
    };
  }, [api, messageId, attempt, enabled]);

  const reload = useCallback(() => {
    setAttempt((current) => current + 1);
  }, []);

  return { state, reload };
}
