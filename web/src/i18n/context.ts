import { createContext, use } from 'react';

import type { Messages } from './en';
import type { Locale } from './locales';

export type I18n = {
    locale: Locale;
    t: Messages;
    setLocale: (locale: Locale) => void;
    format: {
        number: (value: number, options?: Intl.NumberFormatOptions) => string;
        /** Memory and disk, in multiples of 1024; a `signed` growth reads `+180 MB`. */
        bytes: (bytes: number, options?: { signed?: boolean }) => string;
        /** Bytes per second, as bits per second in multiples of 1000. */
        bitRate: (bytesPerSecond: number) => string;
        /** `182.0` reads `182%`. */
        percent: (percent: number) => string;
        milliseconds: (ms: number) => string;
        dateTime: (date: Date, options?: Intl.DateTimeFormatOptions) => string;
        duration: (seconds: number) => string;
        /** Play time, in hours past the first: `2h 14m`, `312h`; a `signed` difference `+2h 14m`. */
        playTime: (ms: number, options?: { signed?: boolean }) => string;
        /** An API timestamp (RFC 3339) as tables show it, in local time. */
        timestamp: (rfc3339: string, options?: { seconds?: boolean }) => string;
    };
};

// Apart from the catalog and the provider: a hot reload of either during development would make
// a second context, which the provider already on the page does not fill.
export const I18nContext = createContext<I18n | null>(null);

export const useI18n = () => {
    const i18n = use(I18nContext);
    if (i18n === null) {
        throw new Error('useI18n must be used inside <I18nProvider>');
    }
    return i18n;
};
