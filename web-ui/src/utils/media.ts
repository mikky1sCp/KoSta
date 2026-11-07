import { MEDIA_MAX_BYTES, isAllowedMediaType, type Api } from '../api';
import type { Message } from '../types';
import { formatBytes } from './format';

export type MediaKind = 'image' | 'video' | 'archive' | 'document' | 'file';

export function isImageMimeType(mime: string | null | undefined): boolean {
  return typeof mime === 'string' && mime.toLowerCase().startsWith('image/');
}

export function isImageFile(file: File): boolean {
  return isImageMimeType(file.type);
}

/** Группа вложения — по ней выбираются иконка и подпись. */
export function mediaKind(mime: string | null | undefined): MediaKind {
  const type = (mime ?? '').toLowerCase();
  if (type.startsWith('image/')) {
    return 'image';
  }
  if (type.startsWith('video/')) {
    return 'video';
  }
  if (type === 'application/zip' || type === 'application/x-zip-compressed') {
    return 'archive';
  }
  if (type === 'application/pdf' || type.startsWith('text/')) {
    return 'document';
  }
  return 'file';
}

/** Подпись типа вложения: «Изображение», «PDF», «Архив»… */
export function mediaTypeLabel(mime: string | null | undefined): string {
  switch (mediaKind(mime)) {
    case 'image':
      return 'Изображение';
    case 'video':
      return 'Видео';
    case 'archive':
      return 'Архив';
    case 'document':
      return (mime ?? '').toLowerCase() === 'application/pdf' ? 'PDF' : 'Текст';
    default:
      return 'Файл';
  }
}

/**
 * Строка для списка чатов. У медиа без подписи сервер присылает пустой
 * `text` — показываем имя файла или нейтральное «Файл»/«Фото».
 */
export function messagePreview(
  message: Pick<Message, 'text' | 'is_media' | 'media_name' | 'media_type'>,
): string {
  const text = message.text.trim();
  if (text.length > 0) {
    return text;
  }
  if (!message.is_media) {
    return '';
  }
  const name = message.media_name?.trim() ?? '';
  if (name.length > 0) {
    return name;
  }
  return mediaKind(message.media_type) === 'image' ? 'Фото' : 'Файл';
}

/** Клиентская проверка вложения до загрузки. `null` — файл можно отправлять. */
export function validateMediaFile(file: File): string | null {
  if (file.size === 0) {
    return 'Файл пустой — выберите другой.';
  }
  if (file.size > MEDIA_MAX_BYTES) {
    return `Файл больше ${formatBytes(MEDIA_MAX_BYTES)} — уменьшите его или выберите другой.`;
  }
  const type = (file.type || '').toLowerCase();
  if (!isAllowedMediaType(type)) {
    return 'Этот тип файла не поддерживается. Можно отправлять изображения (PNG, JPEG, WebP, GIF), PDF, TXT, ZIP и MP4.';
  }
  return null;
}

/**
 * Скачивание вложения: авторизованный fetch → blob → временная ссылка.
 * Прямой `<a href="/api/media/...">` не работает — токен живёт только в заголовке.
 */
export async function downloadMedia(api: Api, message: Message): Promise<void> {
  const blob = await api.fetchMediaBlob(message.id);
  const url = URL.createObjectURL(blob);
  try {
    const link = document.createElement('a');
    link.href = url;
    link.download = message.media_name?.trim() || `kosta-${message.id}`;
    link.rel = 'noopener';
    link.style.display = 'none';
    document.body.appendChild(link);
    link.click();
    link.remove();
  } finally {
    // Даём браузеру начать скачивание, прежде чем освободить blob-url.
    window.setTimeout(() => URL.revokeObjectURL(url), 10_000);
  }
}
