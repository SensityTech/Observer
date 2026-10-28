import { useCallback, useState } from 'react';

// Read before the first paint by public/theme.js, which must use the same key and values.
const STORAGE_KEY = 'observer.theme';

export type Theme = 'dark' | 'light';

const currentTheme = (): Theme => (document.documentElement.dataset.theme === 'light' ? 'light' : 'dark');

export const useTheme = () => {
    const [theme, setThemeState] = useState<Theme>(currentTheme);

    const setTheme = useCallback((next: Theme) => {
        const root = document.documentElement;
        // Every colour flips at once: hover transitions would otherwise fade a few late.
        root.classList.add('is-theming');
        root.dataset.theme = next;
        requestAnimationFrame(() => requestAnimationFrame(() => root.classList.remove('is-theming')));
        setThemeState(next);
        try {
            localStorage.setItem(STORAGE_KEY, next);
        } catch {
            // Private browsing: the choice lasts for this page only.
        }
    }, []);

    return { theme, setTheme };
};
