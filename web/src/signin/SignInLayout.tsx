import { Link } from '@tanstack/react-router';
import { Check, ChevronDown, Moon, Sun } from 'lucide-react';
import type { ReactNode } from 'react';

import { buttonVariants } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';
import { LOCALES, type Locale } from '../i18n/locales';
import { cn } from '../lib/cn';
import { useTheme } from '../theme/theme';
import { Stage } from './Stage';

// The page lives minutes, not across a new year.
const YEAR = new Date().getFullYear();

type SignInLayoutProps = {
    eyebrow: string;
    title: string;
    tone: string;
    lead: string;
    children: ReactNode;
};

/**
 * The layout of the pages seen before the panel: a ruled form rail on the left, following the
 * theme, and the always dark stage on the right (under the form on narrow screens).
 */
export function SignInLayout({ eyebrow, title, tone, lead, children }: SignInLayoutProps) {
    const { t } = useI18n();

    return (
        <div className="signin">
            <header className="signin-head-row">
                <div className="signin-rail signin-head">
                    <Link to="/" className="signin-logo" aria-label={t.signIn.home}>
                        <svg viewBox="0 0 24 24" width="28" height="28" aria-hidden="true">
                            <rect width="24" height="24" rx="6" fill="var(--ink)" />
                            <circle cx="12" cy="12" r="6.25" fill="none" stroke="var(--bg)" strokeWidth="2" />
                            <circle cx="12" cy="12" r="2.25" fill="var(--accent)" />
                        </svg>
                        <span>{t.app.name}</span>
                    </Link>
                    <fieldset className="signin-prefs" aria-label={t.signIn.preferences}>
                        <LanguagePicker />
                        <ThemeToggle />
                    </fieldset>
                </div>
            </header>

            <main className="signin-rail signin-framed signin-frame" id="content">
                <div className="signin-body">
                    <div className="signin-intro">
                        <span className="signin-eyebrow">{eyebrow}</span>
                        <h1 className="signin-title">
                            {title} <span className="signin-tone">{tone}</span>
                        </h1>
                        <p className="signin-lead">{lead}</p>
                    </div>
                    {children}
                </div>
            </main>

            <Stage />

            <footer className="signin-foot-row">
                <div className="signin-rail signin-framed signin-foot">
                    <span>{t.signIn.copyright(YEAR)}</span>
                </div>
            </footer>
        </div>
    );
}

/** One button whose icon and name say what it will do. */
function ThemeToggle() {
    const { t } = useI18n();
    const { theme, setTheme } = useTheme();
    const Icon = theme === 'dark' ? Sun : Moon;

    return (
        <button
            type="button"
            className={buttonVariants({ tone: 'ghost', size: 'icon' })}
            aria-label={theme === 'dark' ? t.theme.switchToLight : t.theme.switchToDark}
            onClick={() => setTheme(theme === 'dark' ? 'light' : 'dark')}
        >
            <Icon size={16} strokeWidth={1.5} aria-hidden="true" />
        </button>
    );
}

function LanguagePicker() {
    const { t, locale, setLocale } = useI18n();
    const current = LOCALES[locale];
    const locales = Object.entries(LOCALES) as [Locale, (typeof LOCALES)[Locale]][];

    const choose = (next: Locale) => {
        document.getElementById('signin-languages')?.hidePopover();
        setLocale(next);
    };

    return (
        <div className="signin-lang">
            <button
                type="button"
                className={cn(buttonVariants({ tone: 'ghost' }), 'signin-lang-button gap-1.5 px-2.5')}
                popoverTarget="signin-languages"
                aria-controls="signin-languages"
                onClick={(event) => {
                    // Placed before it opens: under the button, its right edge on the button's.
                    const box = event.currentTarget.getBoundingClientRect();
                    const menu = document.getElementById('signin-languages');
                    if (menu !== null) {
                        menu.style.top = `${box.bottom + 8}px`;
                        menu.style.right = `${document.documentElement.clientWidth - box.right}px`;
                    }
                }}
            >
                <Flag />
                <span className="sr-only">{t.language.current(current.label)}</span>
                <span aria-hidden="true">{current.short}</span>
                <ChevronDown size={14} strokeWidth={1.5} className="signin-chevron" aria-hidden="true" />
            </button>
            <div
                id="signin-languages"
                popover="auto"
                className="signin-lang-menu"
                aria-label={t.language.menu}
            >
                {locales.map(([code, { label }]) => (
                    <button
                        key={code}
                        type="button"
                        lang={code}
                        className="signin-menu-item"
                        aria-current={code === locale}
                        onClick={() => choose(code)}
                    >
                        <Flag />
                        {label}
                        {code === locale && (
                            <Check size={16} strokeWidth={1.5} className="signin-check" aria-hidden="true" />
                        )}
                    </button>
                ))}
            </div>
        </div>
    );
}

/** English is the only language so far; its mark is the Union Jack, in its own colours. */
function Flag() {
    return (
        <span className="signin-flag" aria-hidden="true">
            <svg viewBox="0 0 60 30" preserveAspectRatio="xMidYMid slice" focusable="false">
                <path fill="var(--flag-blue)" d="M0 0h60v30H0z" />
                <path stroke="var(--flag-white)" strokeWidth="6" d="M0 0l60 30M60 0L0 30" />
                <path
                    fill="var(--flag-red)"
                    d="M0 0l30 15-.894 1.789L-.894 1.789zM30 15l30 15 .894-1.789-30-15zM30 15L60 0l-.894-1.789-30 15zM0 30l30-15 .894 1.789-30 15z"
                />
                <path stroke="var(--flag-white)" strokeWidth="10" d="M30 0v30M0 15h60" />
                <path stroke="var(--flag-red)" strokeWidth="6" d="M30 0v30M0 15h60" />
            </svg>
        </span>
    );
}
