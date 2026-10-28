import { Link } from '@tanstack/react-router';

import { useI18n } from '../i18n/I18nProvider';

export function NotFoundPage() {
    const { t } = useI18n();
    return (
        <div className="flex flex-col items-start gap-4">
            <h1 className="page-title">{t.errors.notFound}</h1>
            <Link to="/" className="btn btn-secondary">
                {t.errors.backToOverview}
            </Link>
        </div>
    );
}
