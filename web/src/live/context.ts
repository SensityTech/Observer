import { createContext, use, useEffect, useRef, useState, useSyncExternalStore } from 'react';

import type { Topic } from '../api/generated/Topic';
import type { Realtime, SnapshotOf, TopicHandlers } from './realtime';

export const LiveContext = createContext<Realtime | null>(null);

export const useRealtime = () => {
    const live = use(LiveContext);
    if (live === null) {
        throw new Error('useRealtime must be used inside <LiveContext>');
    }
    return live;
};

export const useLinkState = () => {
    const live = useRealtime();
    return useSyncExternalStore(live.subscribe, live.getState);
};

/**
 * Follows a topic while the component is mounted, and while `enabled`: a topic that costs the server
 * to feed is followed only while what shows it is on. The handlers may change on every render.
 */
export const useTopic = <T extends Topic>(topic: T, handlers: TopicHandlers<T>, enabled = true) => {
    const live = useRealtime();
    const latest = useRef(handlers);
    useEffect(() => {
        latest.current = handlers;
    });
    useEffect(() => {
        if (!enabled) {
            return;
        }
        return live.watch<T>(topic, {
            snapshot: (data) => latest.current.snapshot?.(data),
            event: (data) => latest.current.event?.(data),
            refused: () => latest.current.refused?.(),
            cleared: () => latest.current.cleared?.(),
            since: () => latest.current.since?.() ?? null,
        });
    }, [live, topic, enabled]);
};

/** A topic's latest snapshot, or null while none holds: until one arrives, and while the link is down. */
export const useSnapshot = <T extends Topic>(topic: T) => {
    const [snapshot, setSnapshot] = useState<SnapshotOf<T> | null>(null);
    useTopic(topic, {
        snapshot: setSnapshot,
        refused: () => setSnapshot(null),
        cleared: () => setSnapshot(null),
    });
    return snapshot;
};
