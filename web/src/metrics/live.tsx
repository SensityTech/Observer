import { createContext, use, useMemo, useState, type ReactNode } from 'react';

import type { LiveSample } from '../api/generated/LiveSample';
import { useTopic } from '../live/context';
import { keepLive } from './merge';

type LiveMetrics = {
    /** How things stand: none before the first sample, nor while the link is down. */
    latest: LiveSample | null;
    /** The newest samples, oldest first, which still fill the trends while the link is down. */
    recent: LiveSample[];
    /** The session does not hold the dashboard permission. */
    refused: boolean;
};

const LiveMetricsContext = createContext<LiveMetrics | null>(null);

/** The one `metrics.live` watcher of the Overview, which its tiles and chart share. */
export function LiveMetricsProvider({ children }: { children: ReactNode }) {
    const [recent, setRecent] = useState<LiveSample[]>([]);
    const [refused, setRefused] = useState(false);
    const [cleared, setCleared] = useState(false);
    useTopic('metrics.live', {
        snapshot: ({ latest }) => {
            setRefused(false);
            setCleared(false);
            if (latest !== null) {
                setRecent((kept) => keepLive(kept, latest));
            }
        },
        event: (sample) => setRecent((kept) => keepLive(kept, sample)),
        refused: () => {
            setRefused(true);
            setRecent([]);
        },
        cleared: () => setCleared(true),
    });
    const value = useMemo(
        () => ({ latest: cleared ? null : (recent.at(-1) ?? null), recent, refused }),
        [recent, refused, cleared],
    );
    return <LiveMetricsContext value={value}>{children}</LiveMetricsContext>;
}

export const useLiveMetrics = () => {
    const live = use(LiveMetricsContext);
    if (live === null) {
        throw new Error('useLiveMetrics must be used inside <LiveMetricsProvider>');
    }
    return live;
};
