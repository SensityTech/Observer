import { en, type Messages } from './en';

// English is the default whatever the browser's language: the operator picks another one explicitly.
export const DEFAULT_LOCALE = 'en';

export const LOCALES = {
    en: { label: 'English', messages: en },
} satisfies Record<string, { label: string; messages: Messages }>;

export type Locale = keyof typeof LOCALES;

export const isLocale = (value: unknown): value is Locale =>
    typeof value === 'string' && Object.hasOwn(LOCALES, value);
