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

/** Follows a topic while the component is mounted. The handlers may change on every render. */
export const useTopic = <T extends Topic>(topic: T, handlers: TopicHandlers<T>) => {
    const live = useRealtime();
    const latest = useRef(handlers);
    useEffect(() => {
        latest.current = handlers;
    });
    useEffect(
        () =>
            live.watch<T>(topic, {
                snapshot: (data) => latest.current.snapshot?.(data),
                event: (data) => latest.current.event?.(data),
                refused: () => latest.current.refused?.(),
            }),
        [live, topic],
    );
};

/** A topic's latest snapshot, or null until one arrives. */
export const useSnapshot = <T extends Topic>(topic: T) => {
    const [snapshot, setSnapshot] = useState<SnapshotOf<T> | null>(null);
    useTopic(topic, { snapshot: setSnapshot });
    return snapshot;
};
