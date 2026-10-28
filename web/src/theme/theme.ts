import { useCallback, useState } from 'react';

// Read before the first paint by public/theme.js, which must use the same key and values.
const STORAGE_KEY = 'observer.theme';

export type Theme = 'dark' | 'light';

const currentTheme = (): Theme => (document.documentElement.dataset.theme === 'light' ? 'light' : 'dark');

export const useTheme = () => {
    const [theme, setThemeState] = useState<Theme>(currentTheme);

    const setTheme = useCallback((next: Theme) => {
        document.documentElement.dataset.theme = next;
        setThemeState(next);
        try {
            localStorage.setItem(STORAGE_KEY, next);
        } catch {
            // Private browsing: the choice lasts for this page only.
        }
    }, []);

    return { theme, setTheme };
};
