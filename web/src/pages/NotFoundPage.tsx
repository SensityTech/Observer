import { Link } from '@tanstack/react-router';

import { buttonVariants } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';

export function NotFoundPage() {
    const { t } = useI18n();
    return (
        <main className="mx-auto flex min-h-dvh w-full max-w-dialog flex-col items-start justify-center gap-4 px-4">
            <h1 className="page-title">{t.notFound.title}</h1>
            <Link to="/" className={buttonVariants()}>
                {t.notFound.back}
            </Link>
        </main>
    );
}
