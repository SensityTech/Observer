import { QueryClient } from '@tanstack/react-query';
import { act, cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { comparisonQuery, olderStartsQuery, resourceQuery, startsQuery } from '../api/changes';
import type { ChangesLive } from '../api/generated/ChangesLive';
import type { SnapshotSummary } from '../api/generated/SnapshotSummary';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { LATEST } from './fixtures';
import { REFRESH_MS, refreshMoved, useChangesLive, type Moved } from './live';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const HASHING: SnapshotSummary = { ...LATEST, state: 'hashing' };
const CHECK: SnapshotSummary = {
    ...LATEST,
    id: 9,
    kind: 'check',
    state: 'hashing',
    cause: null,
    by: null,
    counts: null,
};

/** `server.changes` as the test says it, each refresh `useChangesLive` asks for kept in `moved`. */
const follow = async () => {
    let socket: SocketEvents | undefined;
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: () => {}, close: () => {} };
        },
        refetch: () => {},
    });
    const moved: Moved[] = [];
    function Follower() {
        useChangesLive((what) => moved.push(what));
        return null;
    }
    render(
        <LiveContext value={live}>
            <Follower />
        </LiveContext>,
    );
    live.start();
    await settle();
    const say = (type: 'snapshot' | 'event', data: ChangesLive) =>
        act(() => socket?.message(JSON.stringify({ type, topic: 'server.changes', data })));
    act(() => socket?.message(JSON.stringify({ type: 'hello', connectionId: 1, contract: 1 })));
    return { live, say, moved };
};

describe('useChangesLive', () => {
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it('should gather what a run of events moved into one refresh, three seconds after the first', async () => {
        const { live, say, moved } = await follow();
        say('snapshot', { latest: HASHING, check: CHECK });
        vi.useFakeTimers();

        // While a start hashes, the topic speaks twice a second.
        for (let at = 1; at <= 5; at += 1) {
            say('event', {
                latest: { ...HASHING, counts: HASHING.counts && { ...HASHING.counts, checking: at } },
                check: CHECK,
            });
            vi.advanceTimersByTime(500);
        }
        expect(moved).toEqual([]);
        vi.advanceTimersByTime(REFRESH_MS - 2_500);

        expect(moved).toEqual([{ starts: true, to: new Set([null, LATEST.id]) }]);
        vi.advanceTimersByTime(REFRESH_MS * 2);
        expect(moved).toHaveLength(1);
        live.stop();
    });

    it('should leave the comparisons alone when the latest start moved in nothing they read', async () => {
        const { live, say, moved } = await follow();
        say('snapshot', { latest: HASHING, check: CHECK });
        vi.useFakeTimers();

        say('event', { latest: { ...HASHING, files: 12_000, bytes: 5_000_000_000 }, check: CHECK });
        vi.advanceTimersByTime(REFRESH_MS);
        expect(moved).toEqual([{ starts: true, to: new Set() }]);

        say('event', {
            latest: { ...HASHING, files: 12_000, bytes: 5_000_000_000 },
            check: { ...CHECK, state: 'complete' },
        });
        vi.advanceTimersByTime(REFRESH_MS);
        expect(moved[1]).toEqual({ starts: false, to: new Set(['disk', CHECK.id]) });
        live.stop();
    });

    it('should name both starts when a new one begins, the one before cut short', async () => {
        const { live, say, moved } = await follow();
        say('snapshot', { latest: HASHING, check: null });
        vi.useFakeTimers();

        say('event', { latest: { ...HASHING, id: 4, state: 'listing', counts: null }, check: null });
        vi.advanceTimersByTime(REFRESH_MS);

        expect(moved).toEqual([{ starts: true, to: new Set([null, 3, 4]) }]);
        live.stop();
    });
});

/** A client holding each query as read once, none of them stale yet. */
const client = () => {
    const queryClient = new QueryClient();
    const keys: (readonly unknown[])[] = [
        startsQuery.queryKey,
        olderStartsQuery(40).queryKey,
        comparisonQuery(undefined, undefined).queryKey,
        comparisonQuery(2, 3).queryKey,
        comparisonQuery(undefined, 2).queryKey,
        comparisonQuery(3, 'disk').queryKey,
        resourceQuery(2, 3, 'resources/[qb]/qb-core').queryKey,
        resourceQuery(3, 'disk', 'resources/[qb]/qb-core').queryKey,
    ];
    for (const key of keys) {
        queryClient.setQueryData(key, {});
    }
    const invalidated = () =>
        keys.filter((key) => queryClient.getQueryState(key)?.isInvalidated).map((key) => key.join(' '));
    return { queryClient, invalidated };
};

describe('refreshMoved', () => {
    it('should read again the newest starts and what compares with the latest start, nothing else', () => {
        const { queryClient, invalidated } = client();
        refreshMoved(queryClient, { starts: true, to: new Set([null, 3]) });

        expect(invalidated()).toEqual([
            'changes starts',
            'changes comparison  ',
            'changes comparison 2 3',
            'changes resource 2 3 resources/[qb]/qb-core',
        ]);
    });

    it('should read again only what compares with the disk once a check moved', () => {
        const { queryClient, invalidated } = client();
        refreshMoved(queryClient, { starts: false, to: new Set(['disk']) });

        expect(invalidated()).toEqual([
            'changes comparison 3 disk',
            'changes resource 3 disk resources/[qb]/qb-core',
        ]);
    });
});
