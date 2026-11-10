// Сохранение сессии между перезагрузками.
//
// Токен сервера живёт 7 дней; столько же храним его локально. Запись
// переживает перезагрузку вкладки и перезапуск браузера, поэтому на общем
// устройстве её стоит отключать — на экране входа есть галочка «Запомнить меня».

export interface StoredSession {
  token: string;
  userId: number;
  phone: string;
}

const STORAGE_KEY = 'kosta.session.v1';
/** Совпадает с `TOKEN_TTL_SECS` на сервере. */
const SESSION_TTL_MS = 7 * 24 * 60 * 60 * 1000;

interface StoredEnvelope {
  token: string;
  userId: number;
  phone: string;
  savedAt: number;
}

function isEnvelope(value: unknown): value is StoredEnvelope {
  if (typeof value !== 'object' || value === null) {
    return false;
  }
  const record = value as Record<string, unknown>;
  return (
    typeof record.token === 'string' &&
    record.token.length > 0 &&
    typeof record.userId === 'number' &&
    Number.isFinite(record.userId) &&
    typeof record.phone === 'string' &&
    typeof record.savedAt === 'number' &&
    Number.isFinite(record.savedAt)
  );
}

/** Читает сохранённую сессию; просроченную или битую удаляет. */
export function loadStoredSession(): StoredSession | null {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) {
      return null;
    }
    const parsed = JSON.parse(raw) as unknown;
    if (!isEnvelope(parsed)) {
      window.localStorage.removeItem(STORAGE_KEY);
      return null;
    }
    if (Date.now() - parsed.savedAt > SESSION_TTL_MS) {
      window.localStorage.removeItem(STORAGE_KEY);
      return null;
    }
    return {
      token: parsed.token,
      userId: parsed.userId,
      phone: parsed.phone,
    };
  } catch {
    return null;
  }
}

/** Сохраняет сессию (только если пользователь выбрал «Запомнить меня»). */
export function saveStoredSession(session: StoredSession): void {
  try {
    const envelope: StoredEnvelope = {
      token: session.token,
      userId: session.userId,
      phone: session.phone,
      savedAt: Date.now(),
    };
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(envelope));
  } catch {
    /* localStorage недоступен — сессия останется только в памяти */
  }
}

/** Удаляет сохранённую сессию (выход, истёкший токен). */
export function clearStoredSession(): void {
  try {
    window.localStorage.removeItem(STORAGE_KEY);
  } catch {
    /* localStorage недоступен */
  }
}