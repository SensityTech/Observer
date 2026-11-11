import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, render } from '@testing-library/react';
import { useEffect } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ApiError } from '../api/client';
import type { ResourceRow } from '../api/generated/ResourceRow';
import type { ResourcesView } from '../api/generated/ResourcesView';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { useResources } from './live';
import { RESCAN, underWay } from './sent';

type Followed = ReturnType<typeof useResources>;

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const row = (name: string, changes: Partial<ResourceRow> = {}): ResourceRow => ({
    name,
    folder: '[core]',
    origin: 'server',
    version: null,
    state: 'started',
    failedAt: null,
    warnings: null,
    dependants: [],
    controllable: true,
    last: null,
    asked: null,
    ...changes,
});

const listed = (resources: ResourceRow[], changes: Partial<ResourcesView> = {}): ResourcesView => ({
    stage: 'listed',
    generation: 3,
    resources,
    truncated: false,
    rescan: null,
    rescanning: false,
    ...changes,
});

const LIB = row('ox_lib', { dependants: ['ox_target'] });
const LIST = listed([LIB, row('ox_target')]);

function Follower({ onRender }: { onRender: (followed: Followed) => void }) {
    const followed = useResources();
    useEffect(() => onRender(followed));
    return null;
}

/** The orders the API was sent, each answered when the test says. */
const orders: { path: string; body: unknown; answer: (response: Response) => void }[] = [];

/** `server.resources` as the test says it, and what `useResources` makes of it. */
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
    let latest: Followed | undefined;
    render(
        <QueryClientProvider client={new QueryClient()}>
            <LiveContext value={live}>
                <Follower
                    onRender={(followed) => {
                        latest = followed;
                    }}
                />
            </LiveContext>
        </QueryClientProvider>,
    );
    live.start();
    await settle();
    act(() => socket?.message(JSON.stringify({ type: 'hello', connectionId: 1, contract: 1 })));
    const now = () => {
        if (latest === undefined) {
            throw new Error('the hook never rendered');
        }
        return latest;
    };
    return {
        live,
        now,
        say: (type: 'snapshot' | 'event', data: ResourcesView) =>
            act(() => socket?.message(JSON.stringify({ type, topic: 'server.resources', data }))),
        cut: () => act(() => socket?.close(1006)),
        ask: async (order: (followed: Followed) => void) => {
            act(() => order(now()));
            await settle();
        },
        answer: async (index: number, body: unknown, status = 200) => {
            orders[index]?.answer(Response.json(body, { status }));
            await settle();
        },
    };
};

describe('useResources', () => {
    beforeEach(() => {
        vi.stubGlobal(
            'fetch',
            (url: string, init?: RequestInit) =>
                new Promise<Response>((answer) => {
                    const body: unknown =
                        init?.body === undefined ? undefined : JSON.parse(String(init.body));
                    orders.push({ path: url, body, answer });
                }),
        );
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        orders.length = 0;
    });

    it('should hold the list as the topic says it, whole at each change, and nothing once the link is down', async () => {
        const { live, now, say, cut } = await follow();
        expect(now().view).toBeNull();

        say('snapshot', LIST);
        expect(now().view).toEqual(LIST);

        const stopped = listed([row('ox_lib', { state: 'stopped' }), row('ox_target', { state: 'stopped' })]);
        say('event', stopped);
        expect(now().view).toEqual(stopped);

        cut();
        expect(now().view).toBeNull();
        live.stop();
    });

    it('should show an order from the click until the list has it, whichever of the list and the answer comes first', async () => {
        const { live, now, say, ask, answer } = await follow();
        say('snapshot', LIST);

        await ask((followed) => followed.order('restart', 'ox_lib', true));

        expect(orders).toHaveLength(1);
        expect([orders[0]?.path, orders[0]?.body]).toEqual([
            '/api/servers/default/resources/restart',
            { name: 'ox_lib', dependants: true },
        ]);
        expect(underWay(LIB, now().sent)).toBe('restart');

        // Observer shows the order on the list before it answers the request.
        const asked = { ...LIB, asked: { kind: 'restarted' as const, by: 'ada' } };
        say('event', listed([asked, row('ox_target')]));
        expect(now().sent.has('ox_lib')).toBe(true);

        await answer(0, { delivery: 'written', dependants: ['ox_target'] });
        expect(now().sent.has('ox_lib')).toBe(true);
        expect(now().outcome).toBeNull();

        say('event', LIST);
        expect(now().sent.size).toBe(0);
        expect(underWay(LIB, now().sent)).toBeNull();
        live.stop();
    });

    it('should send one order at a time for a resource', async () => {
        const { live, say, ask } = await follow();
        say('snapshot', LIST);

        await ask((followed) => followed.order('stop', 'ox_lib'));
        await ask((followed) => followed.order('restart', 'ox_lib'));
        await ask((followed) => followed.order('stop', 'ox_target'));

        expect(orders.map((order) => order.path)).toEqual([
            '/api/servers/default/resources/stop',
            '/api/servers/default/resources/stop',
        ]);
        expect(orders.map((order) => order.body)).toEqual([
            { name: 'ox_lib', dependants: false },
            { name: 'ox_target', dependants: false },
        ]);
        live.stop();
    });

    it('should say when FXServer was not reading its console, and keep only the order that waits for it', async () => {
        const { live, now, say, ask, answer } = await follow();
        say('snapshot', LIST);

        await ask((followed) => followed.order('stop', 'ox_lib'));
        await answer(0, { delivery: 'pending', dependants: [] });
        expect(now().outcome).toEqual({ kind: 'pending', verb: 'stop', name: 'ox_lib' });
        expect(now().sent.has('ox_lib')).toBe(true);

        await ask((followed) => followed.order('stop', 'ox_target'));
        expect(now().outcome).toBeNull();
        await answer(1, { delivery: 'busy', dependants: [] });
        expect(now().outcome).toEqual({ kind: 'busy', verb: 'stop', name: 'ox_target' });
        expect(now().sent.has('ox_target')).toBe(false);

        act(() => now().dismiss());
        expect(now().outcome).toBeNull();
        live.stop();
    });

    it('should let go of an order the API refused, and keep why', async () => {
        const { live, now, say, ask, answer } = await follow();
        say('snapshot', LIST);

        await ask((followed) => followed.order('start', 'ox_lib'));
        await answer(0, { code: 'resource_changed' }, 409);

        const outcome = now().outcome;
        expect(outcome).toMatchObject({ kind: 'failed', verb: 'start', name: 'ox_lib' });
        expect(outcome?.kind === 'failed' && outcome.error).toEqual(new ApiError(409, 'resource_changed'));
        expect(now().sent.size).toBe(0);
        live.stop();
    });

    it('should hold a rescan apart from the resources, until the list came again', async () => {
        const { live, now, say, ask, answer } = await follow();
        say('snapshot', LIST);

        await ask((followed) => followed.rescan());
        await ask((followed) => followed.rescan());

        expect(orders).toHaveLength(1);
        expect([orders[0]?.path, orders[0]?.body]).toEqual([
            '/api/servers/default/resources/rescan',
            undefined,
        ]);
        expect(now().sent.has(RESCAN)).toBe(true);
        expect(underWay(LIB, now().sent)).toBeNull();

        await answer(0, { delivery: 'written', dependants: [] });
        const again = listed(LIST.resources, {
            rescan: { at: '2026-11-11T14:05:00Z', by: 'ada', added: [], addedCount: 0, changed: 0 },
        });
        say('event', again);
        expect(now().sent.size).toBe(0);
        expect(now().view?.rescan?.by).toBe('ada');
        live.stop();
    });
});
