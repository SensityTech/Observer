import { createContext, use, useState, type ReactNode } from 'react';

import type { ScheduleStatus } from '../api/generated/ScheduleStatus';
import { useTopic } from '../live/context';

type ScheduleState = {
    /** Null until the topic's snapshot, and while the link is down. */
    schedule: ScheduleStatus | null;
    /** The account holds none of the permissions the topic needs. */
    refused: boolean;
    /**
     * The zone server times read in, kept while the link is down so that they do not move; null
     * before the first snapshot and without the topic.
     */
    zone: string | null;
};

const ScheduleContext = createContext<ScheduleState>({ schedule: null, refused: false, zone: null });

/** Follows `server.schedule` once for the whole shell, as the server's state is followed. */
export function ScheduleProvider({ children }: { children: ReactNode }) {
    const [state, setState] = useState<ScheduleState>({ schedule: null, refused: false, zone: null });
    useTopic('server.schedule', {
        snapshot: (schedule) => setState({ schedule, refused: false, zone: schedule.zone }),
        event: (schedule) => setState({ schedule, refused: false, zone: schedule.zone }),
        refused: () => setState({ schedule: null, refused: true, zone: null }),
        cleared: () => setState((current) => ({ ...current, schedule: null })),
    });
    return <ScheduleContext value={state}>{children}</ScheduleContext>;
}

export const useSchedule = () => use(ScheduleContext);
