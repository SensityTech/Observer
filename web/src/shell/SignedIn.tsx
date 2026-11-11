import { useQuery } from '@tanstack/react-query';
import { useEffect, type ReactNode } from 'react';

import { AdmissionProvider } from '../admission/context';
import { meQuery } from '../api/auth';
import { MeContext } from '../auth/me';
import { AlertsProvider } from '../diagnostics/alerts';
import { useRealtime } from '../live/context';
import { ArtifactsProvider } from '../artifacts/context';
import { ScheduleProvider } from '../schedule/context';
import { ServerStatusProvider } from '../server/context';

/**
 * What every signed-in screen stands on, the panel's shell or a full-screen flow: the account, the
 * live socket, and FXServer's state, schedule, builds, alerts and admission as they change.
 */
export function SignedIn({ children }: { children: ReactNode }) {
    const { data: me } = useQuery(meQuery);
    const live = useRealtime();

    // The socket lives as long as a signed-in screen: signing out unmounts it, and closes it.
    useEffect(() => {
        live.start();
        return () => live.stop();
    }, [live]);

    // Signed out: the route guard is already on its way to the sign-in page.
    if (!me) {
        return null;
    }
    return (
        <MeContext value={me}>
            <ServerStatusProvider>
                <ScheduleProvider>
                    <ArtifactsProvider>
                        <AlertsProvider>
                            <AdmissionProvider>{children}</AdmissionProvider>
                        </AlertsProvider>
                    </ArtifactsProvider>
                </ScheduleProvider>
            </ServerStatusProvider>
        </MeContext>
    );
}
