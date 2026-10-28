import { createContext, use, useEffect, useMemo, useState, type ReactNode } from 'react';

import type { Messages } from './en';
import { formatBytes, formatDateTime, formatNumber } from './format';
import { DEFAULT_LOCALE, LOCALES, isLocale, type Locale } from './locales';

const STORAGE_KEY = 'observer.locale';

type I18n = {
    locale: Locale;
    t: Messages;
    setLocale: (locale: Locale) => void;
    format: {
        number: (value: number, options?: Intl.NumberFormatOptions) => string;
        bytes: (bytes: number) => string;
        dateTime: (date: Date, options?: Intl.DateTimeFormatOptions) => string;
    };
};

const I18nContext = createContext<I18n | null>(null);

const savedLocale = (): Locale => {
    try {
        const saved = localStorage.getItem(STORAGE_KEY);
        return isLocale(saved) ? saved : DEFAULT_LOCALE;
    } catch {
        return DEFAULT_LOCALE;
    }
};

export function I18nProvider({ children }: { children: ReactNode }) {
    const [locale, setLocaleState] = useState<Locale>(savedLocale);

    useEffect(() => {
        document.documentElement.lang = locale;
    }, [locale]);

    const value = useMemo<I18n>(
        () => ({
            locale,
            t: LOCALES[locale].messages,
            setLocale: (next) => {
                setLocaleState(next);
                try {
                    localStorage.setItem(STORAGE_KEY, next);
                } catch {
                    // Private browsing: the choice lasts for this page only.
                }
            },
            format: {
                number: (number, options) => formatNumber(locale, number, options),
                bytes: (bytes) => formatBytes(locale, bytes),
                dateTime: (date, options) => formatDateTime(locale, date, options),
            },
        }),
        [locale],
    );

    return <I18nContext value={value}>{children}</I18nContext>;
}

export const useI18n = () => {
    const i18n = use(I18nContext);
    if (i18n === null) {
        throw new Error('useI18n must be used inside <I18nProvider>');
    }
    return i18n;
};
