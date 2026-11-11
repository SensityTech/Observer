import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { allEntities } from '../api/entities';
import type { EntitiesLive } from '../api/generated/EntitiesLive';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { useEntitiesLive } from './live';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const SWEPT: EntitiesLive = {
    run: '3-52100',
    sweep: 1,
    at: '2026-11-11T20:00:00Z',
    unavailable: null,
    counts: { vehicles: 2, peds: 1, objects: 4, playerPeds: 1 },
    listed: 6,
    truncated: false,
    everyMs: 15_000,
    spaced: false,
    workUs: 900,
    listUs: 120,
    frames: 1,
};

/** `server.entities` as the test says it, and what the hook makes of it. */
const follow = async () => {
    let socket: SocketEvents | undefined;
    const sent: string[] = [];
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: (message) => sent.push(message), close: () => {} };
        },
        refetch: () => {},
    });
    const client = new QueryClient();
    const asked = vi.spyOn(client, 'invalidateQueries');
    const states: (EntitiesLive | null)[] = [];
    function Follower() {
        states.push(useEntitiesLive());
        return null;
    }
    render(
        <QueryClientProvider client={client}>
            <LiveContext value={live}>
                <Follower />
            </LiveContext>
        </QueryClientProvider>,
    );
    live.start();
    await settle();
    act(() => socket?.message(JSON.stringify({ type: 'hello', connectionId: 1, contract: 1 })));
    const say = (type: 'snapshot' | 'event', data: EntitiesLive) =>
        act(() => socket?.message(JSON.stringify({ type, topic: 'server.entities', data })));
    return { say, sent, asked, last: () => states.at(-1) };
};

describe('useEntitiesLive', () => {
    afterEach(cleanup);

    it('should follow the topic, which is what keeps the server listing', async () => {
        const { sent, last } = await follow();

        expect(sent.some((message) => message.includes('server.entities'))).toBe(true);
        expect(last()).toBeNull();
    });

    it('should ask the list again at each new sweep, and not for the state it opens on', async () => {
        const { say, asked, last } = await follow();

        say('snapshot', SWEPT);
        expect(last()).toEqual(SWEPT);
        expect(asked).not.toHaveBeenCalled();

        say('event', { ...SWEPT, sweep: 2 });
        expect(last()?.sweep).toBe(2);
        expect(asked).toHaveBeenCalledExactlyOnceWith(allEntities);

        // The same sweep said twice changes nothing of the list.
        say('event', { ...SWEPT, sweep: 2 });
        expect(asked).toHaveBeenCalledTimes(1);
    });

    it('should ask the list again once the server restarted, whose sweeps count from 1 again', async () => {
        const { say, asked } = await follow();

        say('snapshot', SWEPT);
        say('event', { ...SWEPT, run: '4-900' });

        expect(asked).toHaveBeenCalledExactlyOnceWith(allEntities);
    });
});
