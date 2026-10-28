import { createContext, use, useState, type ReactNode } from 'react';

import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import { useTopic } from '../live/context';

type ArtifactsState = {
    /** Null until the topic's snapshot, and while the link is down. */
    status: ArtifactsStatus | null;
    /** The account holds none of the permissions the topic needs. */
    refused: boolean;
};

const ArtifactsContext = createContext<ArtifactsState>({ status: null, refused: false });

/** Follows `server.artifacts` once for the whole shell, as the schedule is followed. */
export function ArtifactsProvider({ children }: { children: ReactNode }) {
    const [state, setState] = useState<ArtifactsState>({ status: null, refused: false });
    useTopic('server.artifacts', {
        snapshot: (status) => setState({ status, refused: false }),
        event: (status) => setState({ status, refused: false }),
        refused: () => setState({ status: null, refused: true }),
        cleared: () => setState((current) => ({ ...current, status: null })),
    });
    return <ArtifactsContext value={state}>{children}</ArtifactsContext>;
}

export const useArtifacts = () => use(ArtifactsContext);
