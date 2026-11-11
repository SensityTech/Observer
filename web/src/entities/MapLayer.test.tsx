import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { EntityMap } from '../api/generated/EntityMap';
import type { MapBounds } from '../api/generated/MapBounds';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { useEntityLayer, type EntityLayer } from './MapLayer';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const MAP: EntityMap = {
    live: {
        run: '3-52100',
        sweep: 2,
        at: '2026-11-11T20:00:00Z',
        unavailable: null,
        counts: { vehicles: 1, peds: 1, objects: 0, playerPeds: 0 },
        listed: 2,
        truncated: false,
        everyMs: 15_000,
        spaced: false,
        workUs: 900,
        listUs: 100,
        frames: 1,
    },
    kinds: { vehicles: 1, peds: 1, objects: 0 },
    matching: 2,
    points: [
        [12, 120, 2, 100, 100],
        [13, 130, 1, 140, 100],
    ],
    cell: null,
    cells: [],
};

const VIEW = { bounds: { west: 0, east: 900, south: -300, north: 300 }, unitsPerPixel: 1 };

/** The layer's hook over a server that answers `map`, and what it sent over the live link. */
const mount = async (wanted: number | undefined, map: EntityMap = MAP) => {
    const fetch = vi.fn(async (input: RequestInfo | URL) => {
        const url = new URL(String(input), 'http://localhost');
        return url.pathname.endsWith('/map')
            ? Response.json(map)
            : Response.json({ items: [], limits: null, ownersShown: true });
    });
    vi.stubGlobal('fetch', fetch);
    let socket: SocketEvents | undefined;
    const said: { type: string; topic?: string }[] = [];
    const realtime = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: (message) => said.push(JSON.parse(message)), close: () => {} };
        },
        refetch: () => {},
    });
    const held: { layer: EntityLayer | null } = { layer: null };
    function Holder() {
        held.layer = useEntityLayer(true, wanted);
        return null;
    }
    render(
        <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
            <LiveContext value={realtime}>
                <Holder />
            </LiveContext>
        </QueryClientProvider>,
    );
    realtime.start();
    await settle();
    act(() => socket?.message(JSON.stringify({ type: 'hello', connectionId: 1, contract: 1 })));
    await settle();
    const layer = () => held.layer as EntityLayer;
    const asked = () => fetch.mock.calls.map(([input]) => String(input));
    const follows = () =>
        said.filter((message) => message.topic === 'server.entities').map((message) => message.type);
    return { layer, asked, follows };
};

describe('useEntityLayer', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should ask the server for nothing while the layer is off', async () => {
        const { layer, asked, follows } = await mount(undefined);
        act(() => layer().watch(VIEW));
        await settle();

        expect(layer().on).toBe(false);
        expect(follows()).toEqual([]);
        expect(asked()).toEqual([]);
    });

    it('should follow the topic and ask for the part of the map in view once turned on, and stop when off', async () => {
        const { layer, asked, follows } = await mount(undefined);
        act(() => layer().watch(VIEW));
        act(() => layer().turn(true));
        await settle();

        expect(follows()).toEqual(['sub']);
        expect(asked()).toEqual([
            '/api/servers/default/entities/map?cell=50&west=0&east=1024&south=-512&north=512',
        ]);
        await waitFor(() => expect(layer().shown?.points).toHaveLength(2));

        act(() => layer().turn(false));
        await settle();
        expect(follows()).toEqual(['sub', 'unsub']);
        expect(layer().shown).toBeNull();
    });

    it('should ask for the kinds left on only, and draw nothing once none is', async () => {
        const { layer, asked } = await mount(undefined);
        act(() => layer().watch(VIEW));
        act(() => layer().turn(true));
        await settle();

        act(() => layer().toggle('object'));
        await settle();
        expect(asked().at(-1)).toContain('?kinds=vehicle%2Cped&cell=50');

        act(() => layer().toggle('vehicle'));
        act(() => layer().toggle('ped'));
        await settle();
        expect(layer().shown).toBeNull();
    });

    it('should come on for the entity the address names, and read it from the list', async () => {
        const { layer, asked, follows } = await mount(1042);

        expect(layer().on).toBe(true);
        expect(layer().picked).toBe(1042);
        expect(follows()).toEqual(['sub']);
        expect(asked()).toContain('/api/servers/default/entities?search=%231042&sort=netId');
    });

    it('should pick the entity under a press, and let go on a press that reaches none', async () => {
        const { layer } = await mount(undefined);
        act(() => layer().watch(VIEW));
        act(() => layer().turn(true));
        // A press reaches only what the map already drew.
        await waitFor(() => expect(layer().shown).not.toBeNull());

        let zoom: MapBounds | null = null;
        act(() => {
            zoom = layer().press({ x: 138, y: 101, reach: 10 });
        });
        expect(layer().picked).toBe(13);
        expect(zoom).toBeNull();

        act(() => {
            zoom = layer().press({ x: 500, y: 0, reach: 10 });
        });
        expect(layer().picked).toBeNull();
        expect(zoom).toBeNull();
    });

    it('should give the cell under a press to zoom into, when the map counts by square', async () => {
        const { layer } = await mount(undefined, { ...MAP, points: [], cell: 200, cells: [[0, 0, 840, 3]] });
        act(() => layer().watch(VIEW));
        act(() => layer().turn(true));
        await waitFor(() => expect(layer().shown).not.toBeNull());

        let zoom: MapBounds | null = null;
        act(() => {
            zoom = layer().press({ x: 150, y: 20, reach: 10 });
        });

        expect(zoom).toEqual({ west: 0, east: 200, south: 0, north: 200 });
    });
});
