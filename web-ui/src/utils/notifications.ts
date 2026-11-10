// Системные уведомления и короткий звук нового сообщения.
//
// Уведомления показываются только когда вкладка в фоне (иначе они дублировали
// бы интерфейс). Разрешение спрашивается в момент включения, а не при загрузке:
// браузеры блокируют «неожиданные» запросы.

export interface NotificationSettings {
  enabled: boolean;
  sound: boolean;
}

const STORAGE_KEY = 'kosta.notifications.v1';
const DEFAULTS: NotificationSettings = { enabled: false, sound: true };

function isSettings(value: unknown): value is NotificationSettings {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return typeof record.enabled === 'boolean' && typeof record.sound === 'boolean';
}

export function loadNotificationSettings(): NotificationSettings {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      return DEFAULTS;
    }
    const parsed = JSON.parse(raw) as unknown;
    return isSettings(parsed) ? parsed : DEFAULTS;
  } catch {
    return DEFAULTS;
  }
}

export function saveNotificationSettings(settings: NotificationSettings): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
  } catch {
    /* localStorage недоступен */
  }
}

/** Поддерживает ли браузер уведомления. */
export function notificationsSupported(): boolean {
  return typeof window !== 'undefined' && 'Notification' in window;
}

/** Текущее разрешение: 'granted' | 'denied' | 'default' | 'unsupported'. */
export function notificationPermission(): NotificationPermission | 'unsupported' {
  if (!notificationsSupported()) {
    return 'unsupported';
  }
  return Notification.permission;
}

/** Запрашивает разрешение (вызывать только по действию пользователя). */
export async function requestNotificationPermission(): Promise<boolean> {
  if (!notificationsSupported()) {
    return false;
  }
  if (Notification.permission === 'granted') {
    return true;
  }
  if (Notification.permission === 'denied') {
    return false;
  }
  try {
    const result = await Notification.requestPermission();
    return result === 'granted';
  } catch {
    return false;
  }
}

/** Показывает системное уведомление; клик фокусирует окно и зовёт `onClick`. */
export function showNotification(
  title: string,
  body: string,
  onClick?: () => void,
): void {
  if (!notificationsSupported() || Notification.permission !== 'granted') {
    return;
  }
  try {
    const notification = new Notification(title, {
      body,
      tag: 'kosta-message',
      silent: true,
    });
    notification.onclick = () => {
      window.focus();
      onClick?.();
      notification.close();
    };
  } catch {
    /* некоторые браузеры требуют ServiceWorker — молча пропускаем */
  }
}

let audioContext: AudioContext | null = null;

/** Короткий «пинг» через Web Audio: без звуковых файлов и зависимостей. */
export function playPing(): void {
  try {
    const Ctor =
      window.AudioContext ??
      (window as unknown as { webkitAudioContext?: typeof AudioContext })
        .webkitAudioContext;
    if (!Ctor) {
      return;
    }
    audioContext ??= new Ctor();
    const context = audioContext;
    if (context.state === 'suspended') {
      void context.resume();
    }
    const now = context.currentTime;
    const oscillator = context.createOscillator();
    const gain = context.createGain();
    oscillator.type = 'sine';
    oscillator.frequency.setValueAtTime(880, now);
    oscillator.frequency.exponentialRampToValueAtTime(1320, now + 0.09);
    gain.gain.setValueAtTime(0.0001, now);
    gain.gain.exponentialRampToValueAtTime(0.12, now + 0.01);
    gain.gain.exponentialRampToValueAtTime(0.0001, now + 0.18);
    oscillator.connect(gain);
    gain.connect(context.destination);
    oscillator.start(now);
    oscillator.stop(now + 0.2);
  } catch {
    /* звук не критичен */
  }
}