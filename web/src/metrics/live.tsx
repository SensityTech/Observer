import { createContext, use, useMemo, useState, type ReactNode } from 'react';

import type { LiveSample } from '../api/generated/LiveSample';
import { useTopic } from '../live/context';
import { keepLive } from './merge';

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
            if (latest !== null) {
                setRecent((kept) => keepLive(kept, latest));
            }
        },
        event: (sample) => setRecent((kept) => keepLive(kept, sample)),
        refused: () => {
            setRefused(true);
            setRecent([]);
        },
    });
    const value = useMemo(() => ({ latest: recent.at(-1) ?? null, recent, refused }), [recent, refused]);
    return <LiveMetricsContext value={value}>{children}</LiveMetricsContext>;
}

export const useLiveMetrics = () => {
    const live = use(LiveMetricsContext);
    if (live === null) {
        throw new Error('useLiveMetrics must be used inside <LiveMetricsProvider>');
    }
    return live;
};
