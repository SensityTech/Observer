import { createContext, use, useMemo, useState, type ReactNode } from 'react';

import type { LiveSample } from '../api/generated/LiveSample';
import { useTopic } from '../live/context';

/** Two minutes of samples: what a tile's facts and a page fetched a moment ago both need. */
const KEPT = 24;

type LiveMetrics = {
    latest: LiveSample | null;
    /** The newest samples, oldest first. */
    recent: LiveSample[];
    /** The session does not hold the dashboard permission. */
    refused: boolean;
};

const LiveMetricsContext = createContext<LiveMetrics | null>(null);

/** The one `metrics.live` watcher of the Overview, which its tiles and chart share. */
export function LiveMetricsProvider({ children }: { children: ReactNode }) {
    const [recent, setRecent] = useState<LiveSample[]>([]);
    const [refused, setRefused] = useState(false);
    useTopic('metrics.live', {
        snapshot: ({ latest }) => {
            setRefused(false);
            setRecent((kept) => keep(kept, latest));
        },
        event: (sample) => setRecent((kept) => keep(kept, sample)),
        refused: () => {
            setRefused(true);
            setRecent([]);
        },
    });
    const value = useMemo(() => ({ latest: recent.at(-1) ?? null, recent, refused }), [recent, refused]);
    return <LiveMetricsContext value={value}>{children}</LiveMetricsContext>;
}

/** A sample already held, a snapshot after a reconnect say, replaces itself rather than repeat. */
const keep = (kept: LiveSample[], sample: LiveSample | null) => {
    if (sample === null) {
        return kept;
    }
    const older = kept.filter((held) => held.ts < sample.ts);
    return [...older, sample].slice(-KEPT);
};

export const useLiveMetrics = () => {
    const live = use(LiveMetricsContext);
    if (live === null) {
        throw new Error('useLiveMetrics must be used inside <LiveMetricsProvider>');
    }
    return live;
};
