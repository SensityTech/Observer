import { useRouter, type ErrorComponentProps } from '@tanstack/react-router';
import { RotateCw } from 'lucide-react';

import { Button } from '../components/Button';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { CodeLoadError } from '../lib/lazyCode';

/**
 * A page that could not load, most often because Observer stopped answering. A page whose code
 * could not be loaded needs the whole panel loaded again, which is what its button does.
 */
export function RouteError({ error }: ErrorComponentProps) {
    const { t } = useI18n();
    const router = useRouter();
    const code = error instanceof CodeLoadError;

    return (
        <main className="mx-auto flex min-h-dvh w-full max-w-dialog flex-col justify-center gap-4 px-4">
            <Notice tone="danger">{code ? t.failure.code : errorMessage(t, error)}</Notice>
            <Button
                icon={RotateCw}
                className="self-start"
                onClick={() => (code ? window.location.reload() : void router.invalidate())}
            >
                {code ? t.common.reload : t.common.retry}
            </Button>
        </main>
    );
}
