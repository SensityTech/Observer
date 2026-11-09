import { Outlet } from '@tanstack/react-router';
import { Suspense } from 'react';

import { useI18n } from '../i18n/I18nProvider';
import { SignedIn } from './SignedIn';

/** A signed-in flow on the whole screen, without the rail or the players: the server setup. */
export function FocusShell() {
    const { t } = useI18n();
    return (
        <SignedIn>
            <Suspense
                fallback={
                    <div className="grid min-h-dvh place-items-center bg-bg" aria-busy="true">
                        <span className="sr-only">{t.common.loading}</span>
                    </div>
                }
            >
                <Outlet />
            </Suspense>
        </SignedIn>
    );
}
