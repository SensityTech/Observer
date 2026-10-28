// Locale-aware formatting. Message catalogs and components both go through these helpers, so a
// number or a size reads the same way everywhere in a given language.

export type PluralForms = Partial<Record<Intl.LDMLPluralRule, string>> & { other: string };

const pluralRules = new Map<string, Intl.PluralRules>();

export const plural = (locale: string, count: number, forms: PluralForms) => {
    let rules = pluralRules.get(locale);
    if (rules === undefined) {
        rules = new Intl.PluralRules(locale);
        pluralRules.set(locale, rules);
    }
    return forms[rules.select(count)] ?? forms.other;
};

export const formatNumber = (locale: string, value: number, options?: Intl.NumberFormatOptions) =>
    new Intl.NumberFormat(locale, options).format(value);

const BYTE_UNITS = ['byte', 'kilobyte', 'megabyte', 'gigabyte', 'terabyte'] as const;

/** Decimal units (1 GB = 10^9 bytes), as disks and network figures are quoted. */
export const formatBytes = (locale: string, bytes: number) => {
    let value = bytes;
    let unit = 0;
    while (Math.abs(value) >= 1000 && unit < BYTE_UNITS.length - 1) {
        value /= 1000;
        unit += 1;
    }
    // Plain bytes take the long name: the short one reads "512 byte" in English.
    return new Intl.NumberFormat(locale, {
        style: 'unit',
        unit: BYTE_UNITS[unit],
        unitDisplay: unit === 0 ? 'long' : 'short',
        maximumFractionDigits: unit === 0 ? 0 : 1,
    }).format(value);
};

/**
 * A span of time as the panel words it: `12 s`, `15 min`, `2h 14m`, `3h`, `3d 4h`. Intl's own
 * relative formats read `3 hr.`, so the units are written out here and only the numbers localised.
 */
export const formatDuration = (locale: string, totalSeconds: number) => {
    const n = (value: number) => formatNumber(locale, value);
    const seconds = Math.max(0, Math.ceil(totalSeconds));
    if (seconds < 60) {
        return `${n(seconds)} s`;
    }
    if (seconds < 3600) {
        return `${n(Math.ceil(seconds / 60))} min`;
    }
    const minutes = Math.floor(seconds / 60);
    if (minutes < 24 * 60) {
        const hours = Math.floor(minutes / 60);
        return minutes % 60 === 0 ? `${n(hours)}h` : `${n(hours)}h ${n(minutes % 60)}m`;
    }
    const hours = Math.floor(minutes / 60);
    const days = Math.floor(hours / 24);
    return hours % 24 === 0 ? `${n(days)}d` : `${n(days)}d ${n(hours % 24)}h`;
};

const pad = (value: number) => String(value).padStart(2, '0');

/** Tables and logs read dates in ISO order in every language: `2026-10-28 14:02:07`, local time. */
export const formatTimestamp = (date: Date, { seconds = true } = {}) => {
    const day = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
    const time = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
    return seconds ? `${day} ${time}:${pad(date.getSeconds())}` : `${day} ${time}`;
};

export const formatDateTime = (locale: string, date: Date, options?: Intl.DateTimeFormatOptions) =>
    new Intl.DateTimeFormat(locale, {
        dateStyle: 'medium',
        timeStyle: 'short',
        hourCycle: 'h23',
        ...options,
    }).format(date);
