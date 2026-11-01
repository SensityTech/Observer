import type { QueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';

import { startsQuery, type ComparedTo } from '../api/changes';
import type { ChangesLive } from '../api/generated/ChangesLive';
import type { SnapshotSummary } from '../api/generated/SnapshotSummary';
import { useTopic } from '../live/context';

/** While a snapshot hashes the topic speaks twice a second; what it moved is read again at most this often. */
export const REFRESH_MS = 3_000;

/** What `server.changes` moved since the last refresh. */
export type Moved = {
    /** The latest start changed as the list of starts shows it. */
    starts: boolean;
    /** The To of the comparisons to read again: snapshot ids, `disk`, and null for the latest start. */
    to: Set<ComparedTo | null>;
};

const NOTHING: ChangesLive = { latest: null, check: null };

/** What a comparison against the snapshot reads from it: which one, how far its job got, its counts. */
const compared = (snapshot: SnapshotSummary | null) =>
    snapshot === null ? null : JSON.stringify([snapshot.id, snapshot.state, snapshot.counts]);

const movedBetween = (held: ChangesLive, next: ChangesLive): Moved => {
    const to = new Set<ComparedTo | null>();
    if (compared(held.latest) !== compared(next.latest)) {
        to.add(null);
        // A new start cuts the job of the one before short: both read differently now.
        for (const start of [held.latest, next.latest]) {
            if (start !== null) {
                to.add(start.id);
            }
        }
    }
    if (compared(held.check) !== compared(next.check)) {
        to.add('disk');
        for (const check of [held.check, next.check]) {
            if (check !== null) {
                to.add(check.id);
            }
        }
    }
    return { starts: JSON.stringify(held.latest) !== JSON.stringify(next.latest), to };
};

/**
 * The latest start and check of the disk as `server.changes` says them, null until its snapshot and
 * while the link is down. `onMoved` runs at most once every `REFRESH_MS`, after the events that
 * moved something, with all they moved.
 */
export const useChangesLive = (onMoved?: (moved: Moved) => void) => {
    const [live, setLive] = useState<ChangesLive | null>(null);
    const held = useRef<ChangesLive | null>(null);
    const due = useRef<Moved | null>(null);
    const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
    const callback = useRef(onMoved);
    useEffect(() => {
        callback.current = onMoved;
    });
    useEffect(() => () => clearTimeout(timer.current), []);
    const hold = (next: ChangesLive | null) => {
        held.current = next;
        setLive(next);
    };
    useTopic('server.changes', {
        snapshot: hold,
        event: (next) => {
            const moved = movedBetween(held.current ?? NOTHING, next);
            hold(next);
            if (callback.current === undefined || (!moved.starts && moved.to.size === 0)) {
                return;
            }
            due.current =
                due.current === null
                    ? moved
                    : {
                          starts: due.current.starts || moved.starts,
                          to: new Set([...due.current.to, ...moved.to]),
                      };
            timer.current ??= setTimeout(() => {
                timer.current = undefined;
                const run = due.current;
                due.current = null;
                if (run !== null) {
                    callback.current?.(run);
                }
            }, REFRESH_MS);
        },
        refused: () => hold(null),
        cleared: () => hold(null),
    });
    return live;
};

/**
 * Reads again what moved, without cutting short a read already under way: the newest page of
 * starts, and the comparisons and resources whose To moved.
 */
export const refreshMoved = (queryClient: QueryClient, moved: Moved) => {
    const options = { cancelRefetch: false };
    if (moved.starts) {
        void queryClient.invalidateQueries({ queryKey: startsQuery.queryKey, exact: true }, options);
    }
    if (moved.to.size > 0) {
        void queryClient.invalidateQueries(
            {
                queryKey: ['changes'],
                predicate: ({ queryKey }) =>
                    (queryKey[1] === 'comparison' || queryKey[1] === 'resource') &&
                    moved.to.has(queryKey[3] as ComparedTo | null),
            },
            options,
        );
    }
};
