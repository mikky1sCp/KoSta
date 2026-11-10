import { useCallback, useEffect, useState } from 'react';
import {
  loadNotificationSettings,
  notificationPermission,
  playPing,
  requestNotificationPermission,
  saveNotificationSettings,
  showNotification,
  type NotificationSettings,
} from '../utils/notifications';

export interface NotificationsApi {
  settings: NotificationSettings;
  /** Разрешение браузера: 'granted' | 'denied' | 'default' | 'unsupported'. */
  permission: NotificationPermission | 'unsupported';
  /** Включает/выключает уведомления; при включении спрашивает разрешение. */
  setEnabled: (enabled: boolean) => Promise<boolean>;
  setSound: (sound: boolean) => void;
  /** Показывает уведомление, только если вкладка в фоне. */
  notify: (title: string, body: string, onClick?: () => void) => void;
  /** Проигрывает звук, если он включён. */
  ping: () => void;
}

/** Настройки уведомлений: хранятся в localStorage и переживают перезагрузку. */
export function useNotifications(): NotificationsApi {
  const [settings, setSettings] = useState<NotificationSettings>(
    loadNotificationSettings,
  );
  const [permission, setPermission] = useState(notificationPermission);

  useEffect(() => {
    saveNotificationSettings(settings);
  }, [settings]);

  const setEnabled = useCallback(async (enabled: boolean): Promise<boolean> => {
    if (enabled) {
      const granted = await requestNotificationPermission();
      setPermission(notificationPermission());
      if (!granted) {
        return false;
      }
    }
    setSettings((current) => ({ ...current, enabled }));
    return true;
  }, []);

  const setSound = useCallback((sound: boolean) => {
    setSettings((current) => ({ ...current, sound }));
  }, []);

  const notify = useCallback(
    (title: string, body: string, onClick?: () => void) => {
      if (!settings.enabled) {
        return;
      }
      // В активной вкладке уведомление не нужно — сообщение и так видно.
      if (document.visibilityState === 'visible') {
        return;
      }
      showNotification(title, body, onClick);
    },
    [settings.enabled],
  );

  const ping = useCallback(() => {
    if (settings.sound) {
      playPing();
    }
  }, [settings.sound]);

  return { settings, permission, setEnabled, setSound, notify, ping };
}