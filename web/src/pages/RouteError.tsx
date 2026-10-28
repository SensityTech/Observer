import { useRouter, type ErrorComponentProps } from '@tanstack/react-router';
import { RotateCw } from 'lucide-react';

import { Button } from '../components/Button';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';

/** A page that could not load, most often because Observer stopped answering. */
export function RouteError({ error }: ErrorComponentProps) {
    const { t } = useI18n();
    const router = useRouter();

    return (
        <main className="mx-auto flex min-h-dvh w-full max-w-dialog flex-col justify-center gap-4 px-4">
            <Notice tone="danger">{errorMessage(t, error)}</Notice>
            <Button icon={RotateCw} className="self-start" onClick={() => void router.invalidate()}>
                {t.common.retry}
            </Button>
        </main>
    );
}
