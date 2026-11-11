import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
    Outlet,
    RouterProvider,
} from '@tanstack/react-router';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { EntitiesLive } from '../api/generated/EntitiesLive';
import type { EntityItem } from '../api/generated/EntityItem';
import type { EntityPage } from '../api/generated/EntityPage';
import type { Me } from '../api/generated/Me';
import { Permission } from '../api/generated/Permission';
import { MeContext } from '../auth/me';
import { parseEntitiesSearch } from '../entities/search';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { EntitiesPage } from './EntitiesPage';

const PATH = '/api/servers/default/entities';
const words = en.entities;

const me = (permissions: bigint): Me => ({
    id: 2,
    username: 'maxime',
    isOwner: false,
    permissions: permissions.toString(16),
    mustChangePassword: false,
});
const VIEWER = me(Permission.EntitiesView);
const MODERATOR = me(Permission.EntitiesView | Permission.EntitiesDelete | Permission.PlayersMap);

const live = (changes: Partial<EntitiesLive> = {}): EntitiesLive => ({
    run: '3-52100',
    sweep: 4,
    at: new Date().toISOString(),
    unavailable: null,
    counts: { vehicles: 3, peds: 2, objects: 1, playerPeds: 1 },
    listed: 5,
    truncated: false,
    everyMs: 15_000,
    spaced: false,
    workUs: 12_000,
    listUs: 900,
    frames: 9,
    ...changes,
});

const item = (netId: number, changes: Partial<EntityItem> = {}): EntityItem => ({
    netId,
    serial: netId * 10,
    kind: 'vehicle',
    class: 'car',
    model: 0x3c4e2113,
    x: 412.6,
    y: -1208.3,
    z: 32.1,
    owned: false,
    owner: null,
    ownerName: null,
    bucket: 0,
    population: 5,
    resource: null,
    resourcePending: false,
    occupied: false,
    ...changes,
});

const CAR = item(1042, { resource: 'qb-garages', population: 7, owned: true, owner: 12, ownerName: 'Ada' });
const OCCUPIED = item(771, { model: 0x9f05f101, occupied: true, owned: true, owner: 4, ownerName: 'kaito' });
const TRAIN = item(18, { class: 'train', model: 0x3d6aaa9b });
const PED = item(3304, { kind: 'ped', class: null, model: 0x705e61f2, resourcePending: true, bucket: 3 });
const CRATE = item(5120, { kind: 'object', class: null, model: 0xe0faf6d2, resource: 'ox_inventory' });

const page = (changes: Partial<EntityPage> = {}): EntityPage => ({
    live: live(),
    ownersShown: true,
    limits: { pageSize: 100, maxDelete: 100, typedConfirmationFrom: 20, maxMapPoints: 5000, staleSweeps: 3 },
    matching: 5,
    page: 0,
    items: [CAR, OCCUPIED, TRAIN, PED, CRATE],
    facets: {
        kinds: { vehicles: 3, peds: 1, objects: 1 },
        classes: [
            { class: 'car', count: 2 },
            { class: 'train', count: 1 },
        ],
        resources: [
            { resource: null, count: 3 },
            { resource: 'qb-garages', count: 1 },
            { resource: 'ox_inventory', count: 1 },
        ],
        buckets: [
            { bucket: 0, count: 4 },
            { bucket: 3, count: 1 },
        ],
        unowned: 3,
        owned: 2,
        owners: [
            { netId: 12, name: 'Ada', count: 1 },
            { netId: 4, name: 'kaito', count: 1 },
        ],
        models: [{ model: 0x3c4e2113, kind: 'vehicle', count: 2 }],
    },
    ...changes,
});

const json = (body: unknown, status = 200) => Response.json(body, { status });

const api = (listed: EntityPage = page()) =>
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        if (url.pathname === `${PATH}/delete` && init?.method === 'POST') {
            return json({
                results: [
                    { netId: 1042, outcome: 'deleted' },
                    { netId: 5120, outcome: 'occupied' },
                ],
            });
        }
        if (url.pathname === PATH) {
            return json(listed);
        }
        return json({ code: 'not_found', params: {} }, 404);
    });

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** The page at `address`, for `who`, over a server whose list is `fetch`'s; `say` speaks as the topic. */
const open = async (who: Me, fetch = api(), address = '/entities') => {
    vi.stubGlobal('fetch', fetch);
    let socket: SocketEvents | undefined;
    const realtime = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: () => {}, close: () => {} };
        },
        refetch: () => {},
    });
    const root = createRootRoute({ component: Outlet });
    const router = createRouter({
        routeTree: root.addChildren([
            createRoute({
                getParentRoute: () => root,
                path: '/entities',
                validateSearch: parseEntitiesSearch,
                component: EntitiesPage,
            }),
        ]),
        history: createMemoryHistory({ initialEntries: [address] }),
    });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
        <I18nProvider>
            <QueryClientProvider client={client}>
                <LiveContext value={realtime}>
                    <MeContext value={who}>
                        <RouterProvider router={router} />
                    </MeContext>
                </LiveContext>
            </QueryClientProvider>
        </I18nProvider>,
    );
    realtime.start();
    await settle();
    act(() => socket?.message(JSON.stringify({ type: 'hello', connectionId: 1, contract: 1 })));
    await settle();
    const say = (data: EntitiesLive) =>
        act(() => socket?.message(JSON.stringify({ type: 'event', topic: 'server.entities', data })));
    return { fetch, router, say };
};

/** The table's rows: the phone list is in the document too, CSS alone tells them apart. */
const rows = () => within(screen.getByRole('table')).getAllByRole('row').slice(1);
const asked = (fetch: ReturnType<typeof api>) =>
    fetch.mock.calls.map(([input]) => new URL(String(input), 'http://localhost').search);
const boxOf = (row: HTMLElement) => within(row).getByRole<HTMLInputElement>('checkbox');

describe('EntitiesPage', () => {
    beforeAll(() => {
        HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
            this.open = true;
        };
        HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
            this.open = false;
        };
    });

    beforeEach(() => {
        vi.stubGlobal('scrollTo', () => {});
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should list each entity with what it is, what made it and whose game holds it', async () => {
        await open(MODERATOR);

        expect(screen.getByRole('heading', { name: words.count(5) })).toBeTruthy();
        expect(screen.getByText(words.playerPeds(1))).toBeTruthy();
        const [car, occupied, train, ped, crate] = rows();
        expect(car?.textContent).toContain('Car');
        expect(car?.textContent).toContain('0x3C4E2113');
        expect(car?.textContent).toContain('#1042');
        expect(car?.textContent).toContain('qb-garages');
        expect(car?.textContent).toContain('#12');
        expect(car?.textContent).toContain('Ada');
        expect(car?.textContent).toContain('412.6, -1208.3');
        expect(occupied?.textContent).toContain(words.playerInside);
        expect(train?.textContent).toContain('Train');
        expect(train?.textContent).toContain(words.noResource);
        expect(train?.textContent).toContain(words.noOwner);
        expect(ped?.textContent).toContain('Ped');
        expect(ped?.textContent).toContain(words.lookingUp);
        expect(ped?.textContent).toContain(words.bucketOf(3));
        expect(crate?.textContent).toContain('Object');
        expect(crate?.textContent).toContain('ox_inventory');
    });

    it('should tell only that a player owns an entity to who does not see the map, and offer no deletion', async () => {
        await open(
            VIEWER,
            api(
                page({
                    ownersShown: false,
                    items: [{ ...CAR, owner: null, ownerName: null }, TRAIN],
                }),
            ),
        );

        const [car] = rows();
        expect(car?.textContent).toContain(words.someOwner);
        expect(car?.textContent).not.toContain('Ada');
        expect(screen.queryAllByRole('checkbox')).toEqual([]);
        expect(screen.queryByRole('link', { name: /on the map/ })).toBeNull();
        expect(screen.queryByRole('button', { name: /^Delete/ })).toBeNull();
    });

    it('should read its filters in the address and write a new one back to it', async () => {
        const { fetch, router } = await open(MODERATOR, api(), '/entities?kind=vehicle&resource=qb-garages');

        expect(asked(fetch)).toContain('?kinds=vehicle&resource=qb-garages');

        fireEvent.change(screen.getByLabelText(words.class), { target: { value: 'car' } });
        await settle();

        expect(router.state.location.search).toEqual({
            kind: 'vehicle',
            class: 'car',
            resource: 'qb-garages',
        });
        expect(asked(fetch)).toContain('?kinds=vehicle&class=car&resource=qb-garages');
    });

    it('should sort by the column pressed, the other way round when pressed again', async () => {
        const { fetch, router } = await open(MODERATOR);

        fireEvent.click(screen.getByRole('button', { name: words.columns.owner }));
        await settle();
        expect(router.state.location.search).toEqual({ sort: 'owner' });
        expect(asked(fetch)).toContain('?sort=owner');

        fireEvent.click(screen.getByRole('button', { name: words.columns.owner }));
        await settle();
        expect(router.state.location.search).toEqual({ sort: 'owner', descending: true });
        expect(
            screen.getByRole('columnheader', { name: words.columns.owner }).getAttribute('aria-sort'),
        ).toBe('descending');
    });

    it('should never offer a vehicle a player sits in, nor a train', async () => {
        await open(MODERATOR);

        const [car, occupied, train] = rows();
        expect(boxOf(car!).disabled).toBe(false);
        expect(boxOf(occupied!).disabled).toBe(true);
        expect(boxOf(train!).disabled).toBe(true);
        expect(within(occupied!).getByRole<HTMLButtonElement>('button', { name: /^Delete/ }).disabled).toBe(
            true,
        );
    });

    it('should delete what is selected once confirmed, and say what became of each entity', async () => {
        const { fetch } = await open(MODERATOR);
        const [car, , , , crate] = rows();

        fireEvent.click(boxOf(car!));
        fireEvent.click(boxOf(crate!));
        expect(screen.getByText(words.selected(2))).toBeTruthy();
        fireEvent.click(screen.getByRole('button', { name: words.deleteSelected(2) }));

        const dialog = screen.getByRole('alertdialog');
        expect(within(dialog).getByText(words.dialog.title(2))).toBeTruthy();
        expect(within(dialog).getByText(words.dialog.scripted(2))).toBeTruthy();
        fireEvent.click(within(dialog).getByRole('button', { name: words.dialog.confirm(2) }));

        await waitFor(() =>
            expect(screen.getByRole('alert').textContent).toBe(
                'Deleted 1 of 2 entities. 1 had a player inside.',
            ),
        );
        const sent = fetch.mock.calls.find(([, init]) => init?.method === 'POST');
        expect(JSON.parse(String(sent?.[1]?.body))).toEqual({
            run: '3-52100',
            targets: [
                { netId: 1042, serial: 10420, model: 0x3c4e2113 },
                { netId: 5120, serial: 51200, model: 0xe0faf6d2 },
            ],
        });
        expect(screen.queryByText(words.selected(2))).toBeNull();
    });

    it('should empty the selection once the server started again: its net ids name other entities', async () => {
        const { say } = await open(MODERATOR);
        const [car] = rows();

        fireEvent.click(boxOf(car!));
        expect(screen.getByText(words.selected(1))).toBeTruthy();
        say(live({ run: '4-900', sweep: 1 }));
        await settle();

        expect(screen.queryByText(words.selected(1))).toBeNull();
    });

    it('should offer no deletion from a list the server stopped refreshing', async () => {
        const old = live({ at: new Date(Date.now() - 120_000).toISOString() });
        await open(MODERATOR, api(page({ live: old })));

        expect(screen.getByText(/the server stopped answering/)).toBeTruthy();
        const [car] = rows();
        expect(boxOf(car!).disabled).toBe(true);
    });

    it('should say that entities need OneSync when the server runs without it', async () => {
        await open(
            MODERATOR,
            api(
                page({
                    live: live({ unavailable: 'onesync', counts: null, listed: 0 }),
                    items: [],
                    matching: 0,
                }),
            ),
        );

        expect(screen.getByText(words.unavailable.onesync.title)).toBeTruthy();
        expect(screen.queryByRole('table')).toBeNull();
    });
});
