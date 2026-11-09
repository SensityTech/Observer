import { useEffect, useMemo, useState, type ReactNode } from 'react';

import { I18nContext, type I18n } from './context';
import {
    formatBitRate,
    formatBytes,
    formatDateTime,
    formatDuration,
    formatMilliseconds,
    formatNumber,
    formatPercent,
    formatPlayTime,
    formatTimestamp,
} from './format';
import { DEFAULT_LOCALE, LOCALES, isLocale, type Locale } from './locales';

export { useI18n } from './context';

const STORAGE_KEY = 'observer.locale';

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
                bytes: (bytes, options) => formatBytes(locale, bytes, options),
                bitRate: (bytesPerSecond) => formatBitRate(locale, bytesPerSecond),
                percent: (percent) => formatPercent(locale, percent),
                milliseconds: (ms) => formatMilliseconds(locale, ms),
                dateTime: (date, options) => formatDateTime(locale, date, options),
                duration: (seconds) => formatDuration(locale, seconds),
                playTime: (ms, options) => formatPlayTime(locale, ms, options),
                timestamp: (rfc3339, options) => formatTimestamp(new Date(rfc3339), options),
            },
        }),
        [locale],
    );

    return <I18nContext value={value}>{children}</I18nContext>;
}
