import { useState, type FormEvent } from 'react';
import {
  AlertCircle,
  Eye,
  EyeOff,
  LockKeyhole,
  MessageCircle,
  Phone,
} from 'lucide-react';
import { Button } from './ui/Button';
import { TextInput } from './ui/TextInput';
import { cn } from '../utils/classNames';

type AuthMode = 'login' | 'register';

interface AuthScreenProps {
  loading: boolean;
  error: string;
  onSubmit: (phone: string, password: string, register: boolean) => Promise<void>;
}

const PHONE_PATTERN = /^\+?\d{5,20}$/;

export function AuthScreen({ loading, error, onSubmit }: AuthScreenProps) {
  const [mode, setMode] = useState<AuthMode>('login');
  const [phone, setPhone] = useState('');
  const [password, setPassword] = useState('');
  const [showPassword, setShowPassword] = useState(false);
  const [localError, setLocalError] = useState('');

  const isRegister = mode === 'register';

  function switchMode(next: AuthMode) {
    setMode(next);
    setLocalError('');
  }

  function validate(): string | null {
    const normalizedPhone = phone.trim();
    if (!normalizedPhone) {
      return 'Введите номер телефона';
    }
    if (!PHONE_PATTERN.test(normalizedPhone)) {
      return 'Телефон: необязательный «+» и от 5 до 20 цифр';
    }
    if (password.length < 8) {
      return 'Пароль должен быть не короче 8 символов';
    }
    if (password.length > 128) {
      return 'Пароль слишком длинный (максимум 128 символов)';
    }
    return null;
  }

  async function handleSubmit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (loading) {
      return;
    }
    const validationError = validate();
    if (validationError) {
      setLocalError(validationError);
      return;
    }
    setLocalError('');
    await onSubmit(phone.trim(), password, isRegister);
  }

  const message = localError || error;

  return (
    <div className="app-shell flex min-h-[100dvh] w-full items-center justify-center px-4 py-8">
      <div className="animate-fade-in-up w-full max-w-md">
        <div className="mb-6 flex flex-col items-center text-center">
          <span className="mb-4 flex h-16 w-16 items-center justify-center rounded-2xl bg-accent text-white">
            <MessageCircle className="h-8 w-8" strokeWidth={2.2} aria-hidden="true" />
          </span>
          <h1 className="brand-text text-3xl font-bold tracking-tight">
            KoSta<span className="text-accent">.</span>
          </h1>
          <p className="mt-1 text-sm text-muted">
            {isRegister
              ? 'Создайте аккаунт и начните общаться'
              : 'Войдите, чтобы продолжить общение'}
          </p>
        </div>

        <div className="rounded-3xl border border-line bg-panel p-6 shadow-lg sm:p-8">
          <div
            role="tablist"
            aria-label="Вход или регистрация"
            className="mb-6 grid grid-cols-2 gap-1 rounded-xl bg-panel-soft p-1"
          >
            <button
              type="button"
              role="tab"
              aria-selected={!isRegister}
              onClick={() => switchMode('login')}
              className={cn(
                'h-9 rounded-lg text-sm font-medium transition-colors',
                !isRegister
                  ? 'bg-panel-strong text-ink shadow-sm'
                  : 'text-muted hover:text-ink',
              )}
            >
              Вход
            </button>
            <button
              type="button"
              role="tab"
              aria-selected={isRegister}
              onClick={() => switchMode('register')}
              className={cn(
                'h-9 rounded-lg text-sm font-medium transition-colors',
                isRegister
                  ? 'bg-panel-strong text-ink shadow-sm'
                  : 'text-muted hover:text-ink',
              )}
            >
              Регистрация
            </button>
          </div>

          <form onSubmit={handleSubmit} className="space-y-4" noValidate>
            <TextInput
              label="Телефон"
              icon={<Phone className="h-4 w-4" aria-hidden="true" />}
              type="tel"
              inputMode="tel"
              autoComplete="tel"
              placeholder="+7 912 345 67 89"
              value={phone}
              onChange={(event) => setPhone(event.target.value)}
              disabled={loading}
              required
            />
            <TextInput
              label="Пароль"
              icon={<LockKeyhole className="h-4 w-4" aria-hidden="true" />}
              type={showPassword ? 'text' : 'password'}
              autoComplete={isRegister ? 'new-password' : 'current-password'}
              placeholder="Минимум 8 символов"
              value={password}
              onChange={(event) => setPassword(event.target.value)}
              disabled={loading}
              required
              trailing={
                <button
                  type="button"
                  onClick={() => setShowPassword((current) => !current)}
                  aria-label={showPassword ? 'Скрыть пароль' : 'Показать пароль'}
                  className="flex h-8 w-8 items-center justify-center rounded-lg text-muted transition-colors hover:bg-panel-strong hover:text-ink"
                >
                  {showPassword ? (
                    <EyeOff className="h-4 w-4" aria-hidden="true" />
                  ) : (
                    <Eye className="h-4 w-4" aria-hidden="true" />
                  )}
                </button>
              }
            />

            <Button type="submit" fullWidth loading={loading}>
              {isRegister ? 'Зарегистрироваться' : 'Войти'}
            </Button>

            {message && (
              <div
                role="alert"
                className="flex items-start gap-2 rounded-xl border border-danger/25 bg-danger/10 px-3 py-2.5 text-sm text-danger"
              >
                <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" aria-hidden="true" />
                <span className="break-words">{message}</span>
              </div>
            )}
          </form>
        </div>

        <p className="mt-4 text-center text-xs text-muted">
          Пароль передаётся по защищённому каналу, токен хранится только в памяти
          вкладки.
        </p>
      </div>
    </div>
  );
}
