import { useI18n } from '../i18n/I18nProvider';

export function Logo() {
    const { t } = useI18n();
    return (
        <span className="logo">
            <svg viewBox="0 0 24 24" width="24" height="24" aria-hidden="true">
                <rect width="24" height="24" rx="6" fill="var(--ink)" />
                <circle cx="12" cy="12" r="6.25" fill="none" stroke="var(--bg)" strokeWidth="2" />
                <circle cx="12" cy="12" r="2.25" fill="var(--accent)" />
            </svg>
            {t.app.name}
        </span>
    );
}
