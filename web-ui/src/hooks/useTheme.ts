import { useCallback, useEffect, useState } from 'react';
import type { Theme } from '../types';

const THEME_STORAGE_KEY = 'kosta-theme';

function readInitialTheme(): Theme {
  try {
    return localStorage.getItem(THEME_STORAGE_KEY) === 'light' ? 'light' : 'dark';
  } catch {
    return 'dark';
  }
}

/**
 * Тема хранится в localStorage (единственное, что переживает перезагрузку),
 * применяется классом `light` на <html>.
 */
export function useTheme(): [Theme, (theme: Theme) => void, () => void] {
  const [theme, setTheme] = useState<Theme>(readInitialTheme);

  useEffect(() => {
    const root = document.documentElement;
    root.classList.toggle('light', theme === 'light');
    root.style.colorScheme = theme;
    try {
      localStorage.setItem(THEME_STORAGE_KEY, theme);
    } catch {
      /* localStorage может быть недоступен */
    }
  }, [theme]);

  const toggleTheme = useCallback(() => {
    setTheme((current) => (current === 'dark' ? 'light' : 'dark'));
  }, []);

  return [theme, setTheme, toggleTheme];
}
