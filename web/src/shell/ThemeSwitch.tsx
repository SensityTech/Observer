import { Moon, Sun } from 'lucide-react';

import { useI18n } from '../i18n/I18nProvider';
import { useTheme, type Theme } from '../theme/theme';

const CHOICES = [
    { theme: 'dark', Icon: Moon },
    { theme: 'light', Icon: Sun },
] as const satisfies readonly { theme: Theme; Icon: typeof Moon }[];

export function ThemeSwitch() {
    const { t } = useI18n();
    const { theme, setTheme } = useTheme();

    return (
        <fieldset className="theme-switch">
            <legend className="sr-only">{t.theme.label}</legend>
            {CHOICES.map(({ theme: choice, Icon }) => (
                <button
                    key={choice}
                    type="button"
                    aria-pressed={theme === choice}
                    onClick={() => setTheme(choice)}
                >
                    <Icon size={14} strokeWidth={1.5} aria-hidden="true" />
                    {t.theme[choice]}
                </button>
            ))}
        </fieldset>
    );
}
