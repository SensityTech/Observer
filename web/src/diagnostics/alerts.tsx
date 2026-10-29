import { createContext, use, useState, type ReactNode } from 'react';

import type { AlertList } from '../api/generated/AlertList';
import { useTopic } from '../live/context';

type AlertsState = {
    /** Null until the topic's snapshot, and while the link is down or the account may not see it. */
    alerts: AlertList | null;
};

const AlertsContext = createContext<AlertsState>({ alerts: null });

/** Follows `server.alerts` once for the whole shell: the Overview's card and the rail's count read it. */
export function AlertsProvider({ children }: { children: ReactNode }) {
    const [state, setState] = useState<AlertsState>({ alerts: null });
    useTopic('server.alerts', {
        snapshot: (alerts) => setState({ alerts }),
        event: (alerts) => setState({ alerts }),
        refused: () => setState({ alerts: null }),
        cleared: () => setState({ alerts: null }),
    });
    return <AlertsContext value={state}>{children}</AlertsContext>;
}

export const useAlerts = () => use(AlertsContext).alerts?.alerts ?? [];
