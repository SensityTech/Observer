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

export const formatDateTime = (locale: string, date: Date, options?: Intl.DateTimeFormatOptions) =>
    new Intl.DateTimeFormat(locale, {
        dateStyle: 'medium',
        timeStyle: 'short',
        hourCycle: 'h23',
        ...options,
    }).format(date);
