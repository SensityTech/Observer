import { createContext, use, useState, type ReactNode } from 'react';

import type { ServerStatus } from '../api/generated/ServerStatus';
import { useTopic } from '../live/context';

type ServerStatusState = {
    /** Null until the topic's snapshot, and while the link is down. */
    status: ServerStatus | null;
    /** The account does not hold the dashboard permission the topic needs. */
    refused: boolean;
};

const ServerStatusContext = createContext<ServerStatusState>({ status: null, refused: false });

/**
 * Follows `server.status` once for the whole shell: each event is the whole status, so the status
 * bar, the controls and the pages read the same one, however late they mount.
 */
export function ServerStatusProvider({ children }: { children: ReactNode }) {
    const [state, setState] = useState<ServerStatusState>({ status: null, refused: false });
    useTopic('server.status', {
        snapshot: (status) => setState({ status, refused: false }),
        event: (status) => setState({ status, refused: false }),
        refused: () => setState({ status: null, refused: true }),
        cleared: () => setState((current) => ({ ...current, status: null })),
    });
    return <ServerStatusContext value={state}>{children}</ServerStatusContext>;
}

export const useServerStatus = () => use(ServerStatusContext);
