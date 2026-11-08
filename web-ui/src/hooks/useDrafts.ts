import { useCallback, useRef } from 'react';

const DRAFTS_STORAGE_KEY = 'kosta.drafts.v1';
/// Максимальная длина сохраняемого черновика — как у сообщения.
const MAX_DRAFT_LENGTH = 4096;

type DraftsStore = Record<number, string>;

/** Читает черновики из localStorage, отбрасывая мусор (чужие ключи, не строки). */
function loadDrafts(): DraftsStore {
  try {
    const raw = window.localStorage.getItem(DRAFTS_STORAGE_KEY);
    if (!raw) {
      return {};
    }
    const parsed = JSON.parse(raw) as unknown;
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
      return {};
    }
    const store: DraftsStore = {};
    for (const [key, value] of Object.entries(parsed)) {
      const chatId = Number(key);
      if (
        Number.isInteger(chatId) &&
        chatId > 0 &&
        typeof value === 'string' &&
        value.length > 0
      ) {
        store[chatId] = value;
      }
    }
    return store;
  } catch {
    return {};
  }
}

export interface DraftsApi {
  /** Черновик чата или пустая строка. */
  get: (chatId: number) => string;
  /** Сохраняет черновик; пустая строка удаляет его. */
  set: (chatId: number, text: string) => void;
  /** Удаляет черновик чата. */
  clear: (chatId: number) => void;
}

/**
 * Черновики сообщений по чатам.
 *
 * Хранятся в localStorage (один ключ, JSON-объект "chatId -> текст"),
 * пишутся на каждый ввод — переключение чата или перезагрузка вкладки
 * больше не теряют набранный текст. localStorage недоступен — молча
 * работаем как «без черновиков».
 */
export function useDrafts(): DraftsApi {
  const storeRef = useRef<DraftsStore | null>(null);

  function ensureLoaded(): DraftsStore {
    if (storeRef.current === null) {
      const loaded = loadDrafts();
      storeRef.current = loaded;
      return loaded;
    }
    return storeRef.current;
  }

  const persist = useCallback((store: DraftsStore) => {
    try {
      window.localStorage.setItem(DRAFTS_STORAGE_KEY, JSON.stringify(store));
    } catch {
      /* localStorage может быть недоступен — черновики просто не сохранятся */
    }
  }, []);

  const get = useCallback((chatId: number): string => {
    return ensureLoaded()[chatId] ?? '';
  }, []);

  const set = useCallback(
    (chatId: number, text: string) => {
      const store = ensureLoaded();
      const trimmed = text.slice(0, MAX_DRAFT_LENGTH);
      if (trimmed.length === 0) {
        if (!(chatId in store)) {
          return;
        }
        delete store[chatId];
      } else {
        store[chatId] = trimmed;
      }
      persist(store);
    },
    [persist],
  );

  const clear = useCallback(
    (chatId: number) => {
      set(chatId, '');
    },
    [set],
  );

  return { get, set, clear };
}