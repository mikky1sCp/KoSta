import type { Participant } from '../types';

const timeFormatter = new Intl.DateTimeFormat('ru-RU', {
  hour: '2-digit',
  minute: '2-digit',
});

const dayMonthFormatter = new Intl.DateTimeFormat('ru-RU', {
  day: 'numeric',
  month: 'long',
});

const dayMonthYearFormatter = new Intl.DateTimeFormat('ru-RU', {
  day: 'numeric',
  month: 'long',
  year: 'numeric',
});

const shortDateFormatter = new Intl.DateTimeFormat('ru-RU', {
  day: '2-digit',
  month: '2-digit',
});

const shortDateYearFormatter = new Intl.DateTimeFormat('ru-RU', {
  day: '2-digit',
  month: '2-digit',
  year: 'numeric',
});

const DAY_MS = 24 * 60 * 60 * 1000;

function startOfDay(date: Date): number {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();
}

function dayDiff(timestampSeconds: number): number {
  const then = startOfDay(new Date(timestampSeconds * 1000));
  const today = startOfDay(new Date());
  return Math.round((today - then) / DAY_MS);
}

export function formatTime(timestampSeconds: number): string {
  return timeFormatter.format(new Date(timestampSeconds * 1000));
}

/** Время последнего сообщения в списке чатов. */
export function formatChatTime(timestampSeconds: number | null): string {
  if (!timestampSeconds) {
    return '';
  }
  const diff = dayDiff(timestampSeconds);
  if (diff <= 0) {
    return formatTime(timestampSeconds);
  }
  if (diff === 1) {
    return 'вчера';
  }
  const date = new Date(timestampSeconds * 1000);
  if (date.getFullYear() === new Date().getFullYear()) {
    return shortDateFormatter.format(date);
  }
  return shortDateYearFormatter.format(date);
}

/** Подпись разделителя дат в переписке. */
export function formatDayLabel(timestampSeconds: number): string {
  const diff = dayDiff(timestampSeconds);
  if (diff === 0) {
    return 'Сегодня';
  }
  if (diff === 1) {
    return 'Вчера';
  }
  const date = new Date(timestampSeconds * 1000);
  if (date.getFullYear() === new Date().getFullYear()) {
    return dayMonthFormatter.format(date);
  }
  return dayMonthYearFormatter.format(date);
}

export function isSameDay(a: number, b: number): boolean {
  return startOfDay(new Date(a * 1000)) === startOfDay(new Date(b * 1000));
}

const BYTE_UNITS = ['Б', 'КБ', 'МБ', 'ГБ'] as const;

/** Размер файла для карточки вложения: «980 Б», «12,4 КБ», «3 МБ». */
export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes < 0) {
    return '—';
  }
  if (bytes < 1024) {
    return `${Math.round(bytes)} Б`;
  }
  let value = bytes / 1024;
  let unitIndex = 1;
  while (value >= 1024 && unitIndex < BYTE_UNITS.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }
  const formatted = value < 10 ? value.toFixed(1).replace('.', ',') : String(Math.round(value));
  return `${formatted} ${BYTE_UNITS[unitIndex]}`;
}

/** Ключ дня для группировки сообщений разделителями. */
export function dayKey(timestampSeconds: number): string {
  const date = new Date(timestampSeconds * 1000);
  return `${date.getFullYear()}-${date.getMonth()}-${date.getDate()}`;
}

export function pluralize(
  count: number,
  one: string,
  few: string,
  many: string,
): string {
  const mod10 = count % 10;
  const mod100 = count % 100;
  if (mod10 === 1 && mod100 !== 11) {
    return one;
  }
  if (mod10 >= 2 && mod10 <= 4 && (mod100 < 12 || mod100 > 14)) {
    return few;
  }
  return many;
}

/** Подпись человека: имя, если задано, иначе телефон. */
export function userLabel(user: {
  display_name?: string | null;
  phone: string;
}): string {
  const name = user.display_name?.trim();
  return name && name.length > 0 ? name : user.phone;
}

/** «печатает…» для личного чата и «+7999… печатает…» для группы. */
export function buildTypingLabel(
  typingUserIds: number[],
  participants: Participant[],
  isGroup: boolean,
): string | null {
  if (typingUserIds.length === 0) {
    return null;
  }
  if (!isGroup) {
    return 'печатает…';
  }
  const names = typingUserIds.map((userId) => {
    const participant = participants.find((item) => item.user_id === userId);
    return participant ? userLabel(participant) : 'Участник';
  });
  if (names.length === 1) {
    return `${names[0]} печатает…`;
  }
  if (names.length === 2) {
    return `${names[0]} и ${names[1]} печатают…`;
  }
  return 'Несколько участников печатают…';
}
