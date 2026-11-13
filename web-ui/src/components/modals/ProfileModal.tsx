import { useRef, useState, type ChangeEvent } from 'react';
import { Camera, LogOut, Moon, Sun, Wifi, WifiOff } from 'lucide-react';
import type { Api } from '../../api';
import { avatarUrl } from '../../api';
import type { Me, Theme, WsStatus } from '../../types';
import type { NotificationsApi } from '../../hooks/useNotifications';
import { getErrorMessage } from '../../utils/errors';
import { Modal } from '../ui/Modal';
import { Avatar } from '../ui/Avatar';
import { Button } from '../ui/Button';
import { TextInput } from '../ui/TextInput';
import { cn } from '../../utils/classNames';

interface ProfileModalProps {
  api: Api;
  me: Me | null;
  fallbackPhone: string;
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
  wsStatus: WsStatus;
  /** Уведомления и звук нового сообщения. */
  notifications: NotificationsApi;
  /** Сохранена ли сессия на этом устройстве. */
  sessionPersisted: boolean;
  onTogglePersistence: (persist: boolean) => void;
  /** Профиль обновился (имя или аватар) — приложение обновляет состояние. */
  onProfileChange: (me: Me) => void;
  onLogout: () => void;
  onClose: () => void;
}

const STATUS_LABEL: Record<WsStatus, string> = {
  connected: 'Подключено',
  connecting: 'Соединение…',
  reconnecting: 'Переподключение…',
  offline: 'Нет соединения',
};

const STATUS_COLOR: Record<WsStatus, string> = {
  connected: 'text-online',
  connecting: 'text-warn',
  reconnecting: 'text-warn',
  offline: 'text-danger',
};

/** PNG/JPEG/WebP — те же типы, что принимает сервер для аватара. */
const AVATAR_ACCEPT = 'image/png,image/jpeg,image/webp';
const AVATAR_MAX_BYTES = 5 * 1024 * 1024;

export function ProfileModal({
  api,
  me,
  fallbackPhone,
  theme,
  onThemeChange,
  wsStatus,
  notifications,
  sessionPersisted,
  onTogglePersistence,
  onProfileChange,
  onLogout,
  onClose,
}: ProfileModalProps) {
  const phone = me?.phone ?? fallbackPhone;
  const [name, setName] = useState(me?.display_name ?? '');
  const [username, setUsername] = useState(me?.username ?? '');
  const [saving, setSaving] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [error, setError] = useState('');
  const [avatarVersion, setAvatarVersion] = useState(0);
  const fileInputRef = useRef<HTMLInputElement>(null);

  async function handleToggleNotifications(enabled: boolean) {
    setError('');
    const ok = await notifications.setEnabled(enabled);
    if (enabled && !ok) {
      setError('Не удалось включить уведомления: разрешение не выдано.');
    }
  }

  async function handleSaveName() {
    if (saving) {
      return;
    }
    setSaving(true);
    setError('');
    try {
      const trimmedUsername = username.trim();
      const updated = await api.updateProfile(
        name,
        trimmedUsername === '' ? null : trimmedUsername,
      );
      onProfileChange(updated);
      setName(updated.display_name ?? '');
      setUsername(updated.username ?? '');
    } catch (e) {
      setError(getErrorMessage(e));
    } finally {
      setSaving(false);
    }
  }

  async function handleAvatarChange(event: ChangeEvent<HTMLInputElement>) {
    const input = event.currentTarget;
    const file = input.files?.[0] ?? null;
    input.value = '';
    if (!file) {
      return;
    }
    if (file.size > AVATAR_MAX_BYTES) {
      setError('Файл больше 5 МБ');
      return;
    }
    setUploading(true);
    setError('');
    try {
      await api.uploadAvatar(file);
      if (me) {
        onProfileChange({ ...me, has_avatar: true });
      }
      // Сбрасываем кэш браузера для картинки.
      setAvatarVersion((current) => current + 1);
    } catch (e) {
      setError(getErrorMessage(e));
    } finally {
      setUploading(false);
    }
  }

  const avatarSrc =
    me?.has_avatar && me
      ? `${avatarUrl(me.user_id)}?v=${avatarVersion}`
      : null;

  return (
    <Modal
      title="Профиль"
      subtitle="Имя, фото и оформление"
      onClose={onClose}
      size="sm"
      footer={
        <Button
          variant="danger"
          fullWidth
          icon={<LogOut className="h-4 w-4" aria-hidden="true" />}
          onClick={onLogout}
        >
          Выйти из аккаунта
        </Button>
      }
    >
      <div className="flex flex-col items-center pb-2 pt-1 text-center">
        <div className="relative">
          <Avatar
            title={me?.display_name || phone || 'KoSta'}
            isGroup={false}
            size="xl"
            src={avatarSrc}
            initialsFrom={me?.display_name ?? null}
          />
          <button
            type="button"
            onClick={() => fileInputRef.current?.click()}
            disabled={uploading}
            aria-label="Загрузить фото профиля"
            title="Загрузить фото"
            className="absolute -bottom-1 -right-1 flex h-8 w-8 items-center justify-center rounded-full border border-line bg-panel text-muted shadow-md transition-colors hover:text-ink disabled:opacity-50"
          >
            <Camera className="h-4 w-4" aria-hidden="true" />
          </button>
          <input
            ref={fileInputRef}
            type="file"
            accept={AVATAR_ACCEPT}
            onChange={handleAvatarChange}
            disabled={uploading}
            tabIndex={-1}
            aria-hidden="true"
            className="hidden"
          />
        </div>

        <p className="mt-3 text-base font-semibold text-ink">
          {me?.display_name?.trim() || phone || 'Без номера'}
        </p>
        <p className="mt-0.5 text-xs text-muted">
          {me?.display_name?.trim() ? `${phone} · ` : ''}
          ID пользователя: {me?.user_id ?? '—'}
        </p>
        {uploading && <p className="mt-1 text-[11px] text-muted">Загружаем фото…</p>}
      </div>

      <div className="mt-4 space-y-4">
        <section className="space-y-2">
          <div>
            <p className="mb-1.5 text-xs font-medium text-muted">Ваше имя</p>
            <TextInput
              aria-label="Ваше имя"
              placeholder="Как вас называть"
              value={name}
              maxLength={64}
              onChange={(event) => setName(event.target.value)}
              disabled={saving}
            />
            <p className="mt-1 text-[11px] text-muted">
              Имя видят собеседники вместо номера телефона.
            </p>
          </div>
          <div>
            <p className="mb-1.5 text-xs font-medium text-muted">@username</p>
            <TextInput
              aria-label="Username"
              placeholder="например, anya"
              value={username}
              maxLength={32}
              onChange={(event) => setUsername(event.target.value)}
              disabled={saving}
              hint="По нему вас можно найти в поиске. 3–32 символа: латиница, цифры и _."
            />
          </div>
          <div className="flex justify-end">
            <Button
              variant="secondary"
              loading={saving}
              disabled={
                name.trim() === (me?.display_name ?? '') &&
                username.trim() === (me?.username ?? '')
              }
              onClick={() => void handleSaveName()}
            >
              Сохранить
            </Button>
          </div>
        </section>

        {error && (
          <p role="alert" className="rounded-xl border border-danger/25 bg-danger/10 px-3 py-2 text-xs text-danger">
            {error}
          </p>
        )}

        <section>
          <p className="mb-1.5 text-xs font-medium text-muted">Тема оформления</p>
          <div className="grid grid-cols-2 gap-1 rounded-xl bg-panel-soft p-1">
            <button
              type="button"
              onClick={() => onThemeChange('dark')}
              aria-pressed={theme === 'dark'}
              className={cn(
                'flex h-9 items-center justify-center gap-1.5 rounded-lg text-sm font-medium transition-colors',
                theme === 'dark'
                  ? 'bg-panel-strong text-ink shadow-sm'
                  : 'text-muted hover:text-ink',
              )}
            >
              <Moon className="h-4 w-4" aria-hidden="true" />
              Тёмная
            </button>
            <button
              type="button"
              onClick={() => onThemeChange('light')}
              aria-pressed={theme === 'light'}
              className={cn(
                'flex h-9 items-center justify-center gap-1.5 rounded-lg text-sm font-medium transition-colors',
                theme === 'light'
                  ? 'bg-panel-strong text-ink shadow-sm'
                  : 'text-muted hover:text-ink',
              )}
            >
              <Sun className="h-4 w-4" aria-hidden="true" />
              Светлая
            </button>
          </div>
        </section>

        <section>
          <p className="mb-1.5 text-xs font-medium text-muted">Соединение</p>
          <div className="flex items-center justify-between rounded-xl border border-line bg-panel-soft px-3 py-2.5 text-sm">
            <span className="flex items-center gap-2 text-muted">
              {wsStatus === 'connected' ? (
                <Wifi className="h-4 w-4" aria-hidden="true" />
              ) : (
                <WifiOff className="h-4 w-4" aria-hidden="true" />
              )}
              WebSocket
            </span>
            <span className={cn('text-xs font-medium', STATUS_COLOR[wsStatus])}>
              {STATUS_LABEL[wsStatus]}
            </span>
          </div>
        </section>

        <section>
          <p className="mb-1.5 text-xs font-medium text-muted">Уведомления</p>
          <div className="space-y-2">
            <label className="flex cursor-pointer items-start gap-2 rounded-xl border border-line bg-panel-soft px-3 py-2.5">
              <input
                type="checkbox"
                checked={notifications.settings.enabled}
                onChange={(event) => void handleToggleNotifications(event.target.checked)}
                className="mt-0.5 h-4 w-4 shrink-0 cursor-pointer accent-[var(--accent)]"
              />
              <span className="min-w-0">
                <span className="block text-sm text-ink">Показывать уведомления</span>
                <span className="mt-0.5 block text-[11px] leading-relaxed text-muted">
                  {notifications.permission === 'unsupported'
                    ? 'Браузер не поддерживает системные уведомления.'
                    : notifications.permission === 'denied'
                      ? 'Разрешение заблокировано в настройках браузера для этого сайта.'
                      : 'Всплывают, когда вкладка свёрнута или неактивна.'}
                </span>
              </span>
            </label>

            <label className="flex cursor-pointer items-start gap-2 rounded-xl border border-line bg-panel-soft px-3 py-2.5">
              <input
                type="checkbox"
                checked={notifications.settings.sound}
                onChange={(event) => notifications.setSound(event.target.checked)}
                className="mt-0.5 h-4 w-4 shrink-0 cursor-pointer accent-[var(--accent)]"
              />
              <span className="min-w-0">
                <span className="block text-sm text-ink">Звук нового сообщения</span>
                <span className="mt-0.5 block text-[11px] leading-relaxed text-muted">
                  Короткий сигнал на входящие сообщения.
                </span>
              </span>
            </label>
          </div>
        </section>

        <section>
          <p className="mb-1.5 text-xs font-medium text-muted">Сессия</p>
          <label className="flex cursor-pointer items-start gap-2 rounded-xl border border-line bg-panel-soft px-3 py-2.5">
            <input
              type="checkbox"
              checked={sessionPersisted}
              onChange={(event) => onTogglePersistence(event.target.checked)}
              className="mt-0.5 h-4 w-4 shrink-0 cursor-pointer accent-[var(--accent)]"
            />
            <span className="min-w-0">
              <span className="block text-sm text-ink">Запомнить меня</span>
              <span className="mt-0.5 block text-[11px] leading-relaxed text-muted">
                Сессия сохраняется на этом устройстве на 7 дней и переживает
                перезагрузку. На общем компьютере лучше выключить.
              </span>
            </span>
          </label>
        </section>

        <p className="rounded-xl bg-panel-soft px-3 py-2.5 text-[11px] leading-relaxed text-muted">
          Выход из аккаунта отзывает токен на сервере и стирает его с этого
          устройства.
        </p>
      </div>
    </Modal>
  );
}