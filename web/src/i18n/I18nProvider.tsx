import { createContext, use, useEffect, useMemo, useState, type ReactNode } from 'react';

import type { Messages } from './en';
import {
    formatBitRate,
    formatBytes,
    formatDateTime,
    formatDuration,
    formatMilliseconds,
    formatNumber,
    formatPercent,
    formatTimestamp,
} from './format';
import { DEFAULT_LOCALE, LOCALES, isLocale, type Locale } from './locales';

const STORAGE_KEY = 'observer.locale';

type I18n = {
    locale: Locale;
    t: Messages;
    setLocale: (locale: Locale) => void;
    format: {
        number: (value: number, options?: Intl.NumberFormatOptions) => string;
        /** Memory and disk, in multiples of 1024. */
        bytes: (bytes: number) => string;
        /** Bytes per second, as bits per second in multiples of 1000. */
        bitRate: (bytesPerSecond: number) => string;
        /** `182.0` reads `182%`. */
        percent: (percent: number) => string;
        milliseconds: (ms: number) => string;
        dateTime: (date: Date, options?: Intl.DateTimeFormatOptions) => string;
        duration: (seconds: number) => string;
        /** An API timestamp (RFC 3339) as tables show it, in local time. */
        timestamp: (rfc3339: string, options?: { seconds?: boolean }) => string;
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
                bitRate: (bytesPerSecond) => formatBitRate(locale, bytesPerSecond),
                percent: (percent) => formatPercent(locale, percent),
                milliseconds: (ms) => formatMilliseconds(locale, ms),
                dateTime: (date, options) => formatDateTime(locale, date, options),
                duration: (seconds) => formatDuration(locale, seconds),
                timestamp: (rfc3339, options) => formatTimestamp(new Date(rfc3339), options),
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
