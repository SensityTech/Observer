import { createContext, use, useState, type ReactNode } from 'react';

import type { AdmissionStatus } from '../api/generated/AdmissionStatus';
import { useTopic } from '../live/context';

type AdmissionState = {
    /** Null until the topic's snapshot, and while the link is down or the account may not see it. */
    admission: AdmissionStatus | null;
};

const AdmissionContext = createContext<AdmissionState>({ admission: null });

/**
 * Follows `server.admission` once for the whole shell: the rail's count of waiting requests and the
 * Allowlist page read it.
 */
export function AdmissionProvider({ children }: { children: ReactNode }) {
    const [state, setState] = useState<AdmissionState>({ admission: null });
    useTopic('server.admission', {
        snapshot: (admission) => setState({ admission }),
        event: (admission) => setState({ admission }),
        refused: () => setState({ admission: null }),
        cleared: () => setState({ admission: null }),
    });
    return <AdmissionContext value={state}>{children}</AdmissionContext>;
}

export const useAdmission = () => use(AdmissionContext).admission;
