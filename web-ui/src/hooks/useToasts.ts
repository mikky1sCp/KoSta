import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { ToastKind } from '../types';

export interface ToastItem {
  id: number;
  kind: ToastKind;
  text: string;
}

export interface ToastApi {
  error: (text: string) => void;
  success: (text: string) => void;
  info: (text: string) => void;
}

const TOAST_TTL_MS = 4_500;
const MAX_TOASTS = 4;

/** Простые тосты: максимум 4 одновременно, автоскрытие через 4.5 с. */
export function useToasts(): {
  toasts: ToastItem[];
  remove: (id: number) => void;
  toast: ToastApi;
} {
  const [toasts, setToasts] = useState<ToastItem[]>([]);
  const nextIdRef = useRef(0);
  const timersRef = useRef(new Map<number, number>());

  const remove = useCallback((id: number) => {
    const timer = timersRef.current.get(id);
    if (timer !== undefined) {
      window.clearTimeout(timer);
      timersRef.current.delete(id);
    }
    setToasts((current) => current.filter((item) => item.id !== id));
  }, []);

  const push = useCallback((kind: ToastKind, text: string) => {
    const id = (nextIdRef.current += 1);
    setToasts((current) => [...current.slice(-(MAX_TOASTS - 1)), { id, kind, text }]);
    const timer = window.setTimeout(() => {
      timersRef.current.delete(id);
      setToasts((current) => current.filter((item) => item.id !== id));
    }, TOAST_TTL_MS);
    timersRef.current.set(id, timer);
  }, []);

  useEffect(() => {
    const timers = timersRef.current;
    return () => {
      timers.forEach((timer) => window.clearTimeout(timer));
      timers.clear();
    };
  }, []);

  const toast = useMemo<ToastApi>(
    () => ({
      error: (text: string) => push('error', text),
      success: (text: string) => push('success', text),
      info: (text: string) => push('info', text),
    }),
    [push],
  );

  return { toasts, remove, toast };
}
