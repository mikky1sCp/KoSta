import { LogOut, Moon, Sun, Wifi, WifiOff } from 'lucide-react';
import type { Me, Theme, WsStatus } from '../../types';
import { Modal } from '../ui/Modal';
import { Avatar } from '../ui/Avatar';
import { Button } from '../ui/Button';
import { cn } from '../../utils/classNames';

interface ProfileModalProps {
  me: Me | null;
  fallbackPhone: string;
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
  wsStatus: WsStatus;
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

export function ProfileModal({
  me,
  fallbackPhone,
  theme,
  onThemeChange,
  wsStatus,
  onLogout,
  onClose,
}: ProfileModalProps) {
  const phone = me?.phone ?? fallbackPhone;

  return (
    <Modal
      title="Профиль"
      subtitle="Аккаунт и оформление"
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
        <Avatar title={phone || 'KoSta'} isGroup={false} size="xl" />
        <p className="mt-3 text-base font-semibold text-ink">
          {phone || 'Без номера'}
        </p>
        <p className="mt-0.5 text-xs text-muted">
          ID пользователя: {me?.user_id ?? '—'}
        </p>
      </div>

      <div className="mt-4 space-y-4">
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

        <p className="rounded-xl bg-panel-soft px-3 py-2.5 text-[11px] leading-relaxed text-muted">
          Токен сессии хранится только в памяти вкладки. После перезагрузки
          страницы потребуется войти заново — это защищает аккаунт на общих
          устройствах.
        </p>
      </div>
    </Modal>
  );
}
