import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { DebugBags } from '../api/generated/DebugBags';
import type { DebugEvents } from '../api/generated/DebugEvents';
import type { DebugHttp } from '../api/generated/DebugHttp';
import type { DebugStatus } from '../api/generated/DebugStatus';
import type { Me } from '../api/generated/Me';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { DebugBagsPage } from './DebugBagsPage';
import { DebugEventsPage } from './DebugEventsPage';
import { DebugHttpPage } from './DebugHttpPage';

vi.mock('@tanstack/react-router', () => ({
    Link: ({
        to,
        params,
        children,
        className,
    }: {
        to: string;
        params?: Record<string, string>;
        children: ReactNode;
        className?: string;
    }) => (
        <a href={to.replace('$id', params?.id ?? '')} className={className}>
            {children}
        </a>
    ),
}));

const words = en.debug;

const OWNER: Me = {
    id: 1,
    username: 'ada',
    isOwner: true,
    permissions: 'fffffff',
    mustChangePassword: false,
};
/** Reads the diagnostics, and nothing else. */
const READER: Me = {
    id: 2,
    username: 'grace',
    isOwner: false,
    permissions: '1000',
    mustChangePassword: false,
};

const COUNTING: DebugStatus = {
    state: 'counting',
    running: true,
    since: '2026-11-10T20:35:00Z',
    reporting: 214,
    silent: ['screenshot-basic', 'webpack'],
    silentCount: 2,
    refused: null,
};

const EVENTS: DebugEvents = {
    status: COUNTING,
    seconds: 300,
    totals: {
        toClients: { count: 56_100, bytes: 2_430_000 },
        fromClients: { count: 7200, bytes: 273_600 },
        local: 93_000,
        handlerUs: 1_800_000,
        refused: 3,
        refusedPlayers: 1,
    },
    series: [
        { at: '2026-11-10T20:49:00Z', seconds: 5, toClients: 31_000, fromClients: 4000 },
        { at: '2026-11-10T20:54:00Z', seconds: 5, toClients: 40_500, fromClients: 4500 },
    ],
    events: [
        {
            name: 'pma-voice:setTalkingOnRadio',
            way: 'toClient',
            resources: ['pma-voice'],
            resourceCount: 1,
            count: 55_800,
            bytes: 2_287_800,
            largest: 44,
            handlerUs: 0,
            senders: [],
        },
        {
            name: 'pma-voice:setTalkingOnRadio',
            way: 'fromClients',
            resources: ['pma-voice'],
            resourceCount: 1,
            count: 7200,
            bytes: 273_600,
            largest: 40,
            handlerUs: 144_000,
            senders: [
                { player: 41, count: 5700, bytes: 216_600 },
                { player: 7, count: 900, bytes: 34_200 },
                { player: 63, count: 600, bytes: 22_800 },
            ],
        },
        {
            name: 'playerDropped',
            way: 'local',
            resources: ['pma-voice', 'hud', 'garage'],
            resourceCount: 22,
            count: 30,
            bytes: 1440,
            largest: 96,
            handlerUs: 900,
            senders: [],
        },
    ],
    resources: [
        {
            resource: 'pma-voice',
            toClients: { count: 55_800, bytes: 2_287_800 },
            fromClients: { count: 7200, bytes: 273_600 },
            local: 10,
            handlerUs: 144_300,
        },
    ],
    players: [{ player: 41, count: 5703, bytes: 216_780, top: 'pma-voice:setTalkingOnRadio' }],
    refused: [{ name: 'garage:giveVehicle', player: 41, resource: 'garage', count: 3 }],
    truncated: false,
};

const ONLINE = {
    players: [
        { netId: 41, player: 12, name: 'Kenji', label: null },
        { netId: 7, player: null, name: null, label: null },
    ],
};

const answer = (data: unknown, status = 200) => Response.json(data, { status });
const OFF = () => answer({ code: 'feature_disabled', params: { feature: 'debug' } }, 404);

/** The API as the pages ask it: each route by its path, the query string apart. */
const serve = (routes: Record<string, (search: string, body: unknown) => Response>) => {
    const asked: { route: string; search: string; body: unknown }[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string, init: RequestInit) => {
            const [path = '', search = ''] = String(input)
                .replace(/^\/api/, '')
                .split('?');
            const route = `${init.method ?? 'GET'} ${path}`;
            const body = typeof init.body === 'string' ? JSON.parse(init.body) : undefined;
            asked.push({ route, search, body });
            return routes[route]?.(search, body) ?? answer({ code: 'not_found', params: {} }, 404);
        }),
    );
    return asked;
};

const open = (page: ReactNode, me: Me = OWNER) => {
    const client = new QueryClient({
        defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    return render(
        <I18nProvider>
            <QueryClientProvider client={client}>
                <MeContext value={me}>{page}</MeContext>
            </QueryClientProvider>
        </I18nProvider>,
    );
};

/** The page's content measured `width` pixels wide, as between the rail and the players' column. */
const contentWide = (width: number) => {
    class Resized {
        observe = vi.fn();
        disconnect = vi.fn();
    }
    vi.stubGlobal('ResizeObserver', Resized);
    vi.spyOn(Element.prototype, 'getBoundingClientRect').mockReturnValue({ width } as DOMRect);
};

/** The cells of a row of a table, as text. */
const cells = (row: HTMLElement | undefined) =>
    within(row as HTMLElement)
        .getAllByRole('cell')
        .map((cell) => cell.textContent);

describe('Debug pages', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        vi.restoreAllMocks();
    });

    it('should say Debug is off with what turning it on changes, and wait for a start once the owner did', async () => {
        let on = false;
        const waiting: DebugEvents = {
            ...EVENTS,
            status: { ...COUNTING, state: 'waiting', since: null, reporting: 0, silent: [], silentCount: 0 },
        };
        const asked = serve({
            'GET /debug/events': () => (on ? answer(waiting) : OFF()),
            'PUT /features/debug': () => {
                on = true;
                return answer({ features: [] });
            },
        });
        const { findByRole, findByText, getByText, queryByRole } = open(<DebugEventsPage />);

        expect(await findByText(words.off.title)).not.toBeNull();
        expect(getByText(words.off.points[0]?.title ?? '')).not.toBeNull();
        expect(queryByRole('button', { name: words.pause })).toBeNull();
        fireEvent.click(await findByRole('button', { name: words.off.turnOn }));

        expect(await findByText(words.waiting.title)).not.toBeNull();
        expect(getByText(words.waiting.running)).not.toBeNull();
        expect(asked.find((call) => call.route === 'PUT /features/debug')?.body).toEqual({ enabled: true });
        expect(queryByRole('table')).toBeNull();
    });

    it('should leave the switch to the owner, and say when observer.toml holds Debug off', async () => {
        serve({ 'GET /debug/events': OFF });
        const reader = open(<DebugEventsPage />, READER);
        expect(await reader.findByText(words.off.ownerOnly)).not.toBeNull();
        expect(reader.queryByRole('button', { name: words.off.turnOn })).toBeNull();
        cleanup();

        serve({
            'GET /debug/events': () =>
                answer({ code: 'feature_disabled', params: { feature: 'debug', held: 'host' } }, 404),
        });
        const locked = open(<DebugEventsPage />);
        expect(await locked.findByText(words.off.locked)).not.toBeNull();
        expect(locked.queryByRole('button', { name: words.off.turnOn })).toBeNull();
    });

    it('should say why the build refused the two lines', async () => {
        const reason = 'scheduler.lua is not a scheduler Observer knows';
        serve({
            'GET /debug/events': () =>
                answer({
                    ...EVENTS,
                    status: { ...COUNTING, state: 'refused', since: null, reporting: 0, refused: reason },
                } satisfies DebugEvents),
        });
        const { findByText, getByRole } = open(<DebugEventsPage />);

        expect(await findByText(words.refused.title)).not.toBeNull();
        expect(await findByText(words.refused.said(reason))).not.toBeNull();
        expect(getByRole('button', { name: words.waiting.turnOff })).not.toBeNull();
    });

    it('should total the events a second, and say under an event who sends it most', async () => {
        const asked = serve({
            'GET /debug/events': () => answer(EVENTS),
            'GET /servers/default/players/online': () => answer(ONLINE),
        });
        const { findByRole, findAllByRole, getByRole, getByText, getAllByRole } = open(<DebugEventsPage />);
        const events = words.events;

        const table = within(await findByRole('table'));
        // Since when, in the panel's own date format, and how many resources report.
        expect(getByText(/^counting since .+ · 214 resources report$/)).not.toBeNull();
        const rows = table.getAllByRole('row').slice(1);
        expect(rows.map((row) => within(row).getAllByRole('cell')[1]?.textContent)).toEqual([
            events.ways.toClient,
            events.ways.fromClients,
            events.ways.local,
        ]);
        // 55,800 in five minutes is 186 a second; 22 resources handle the last one.
        expect(within(rows[0] as HTMLElement).getByText('186')).not.toBeNull();
        expect(within(rows[2] as HTMLElement).getByText(words.resourceCount(22))).not.toBeNull();
        expect(within(rows[2] as HTMLElement).getByText(events.staysHere)).not.toBeNull();
        const name = getAllByRole('button', { name: 'pma-voice:setTalkingOnRadio' })[1] as HTMLElement;
        expect(name.getAttribute('aria-expanded')).toBe('false');
        expect(table.queryByText(events.picked.senders)).toBeNull();
        fireEvent.click(name);

        // What is known of the event opens under its own row, in the table.
        expect(name.getAttribute('aria-expanded')).toBe('true');
        expect(table.getByText(events.picked.senders)).not.toBeNull();
        expect(table.getByText(events.msEach('0.02'))).not.toBeNull();
        // Among those who send it, and again in what was refused: the same client, by name.
        const kenji = await findAllByRole('link', { name: 'Kenji' });
        expect(kenji.map((link) => link.getAttribute('href'))).toEqual(['/players/12', '/players/12']);
        expect(getByText(words.left)).not.toBeNull();
        expect(table.getByText(words.rate('19'))).not.toBeNull();
        expect(getByText(events.refusedCard.title(3, words.windows['5m']))).not.toBeNull();
        expect(asked.some((call) => call.route === 'GET /debug/events' && call.search === 'window=5m')).toBe(
            true,
        );

        fireEvent.click(getByRole('button', { name: words.windows['1m'] }));
        await waitFor(() =>
            expect(
                asked.some((call) => call.route === 'GET /debug/events' && call.search === 'window=1m'),
            ).toBe(true),
        );
        fireEvent.click(getByRole('button', { name: words.pause }));
        expect(getByText(words.paused)).not.toBeNull();
        expect(getByRole('button', { name: words.resume })).not.toBeNull();
        // Its name closes it again.
        fireEvent.click(name);
        expect(table.queryByText(events.picked.senders)).toBeNull();
    });

    it("should put the resource under the event's name where its column does not fit", async () => {
        serve({
            'GET /debug/events': () => answer(EVENTS),
            'GET /servers/default/players/online': () => answer(ONLINE),
        });
        contentWide(1000);
        const roomy = open(<DebugEventsPage />);
        const all = within(await roomy.findByRole('table')).getAllByRole('row');
        expect(within(all[0] as HTMLElement).getAllByRole('columnheader')).toHaveLength(6);
        expect(cells(all[3])[0]).toBe('playerDropped');
        cleanup();

        // On a 1440 px screen with the players' column open, the content is 860 px wide.
        contentWide(860);
        const narrow = open(<DebugEventsPage />);
        const rows = within(await narrow.findByRole('table')).getAllByRole('row');
        expect(within(rows[0] as HTMLElement).getAllByRole('columnheader')).toHaveLength(5);
        expect(cells(rows[3])[0]).toBe(`playerDropped${words.resourceCount(22)}`);
    });

    it('should list the clients and what was refused, and name the resources that do not report', async () => {
        serve({
            'GET /debug/events': () => answer({ ...EVENTS, truncated: true } satisfies DebugEvents),
            'GET /servers/default/players/online': () => answer(ONLINE),
        });
        const { findByRole, getByRole, getByText } = open(<DebugEventsPage />);
        await findByRole('table');

        expect(getByText(words.truncated)).not.toBeNull();
        fireEvent.click(getByRole('button', { name: words.silent(2) }));
        expect(getByText('screenshot-basic, webpack')).not.toBeNull();
        fireEvent.click(getByRole('button', { name: words.events.views.players }));
        expect(within(getByRole('table')).getByText('pma-voice:setTalkingOnRadio')).not.toBeNull();
        fireEvent.click(getByRole('button', { name: words.events.views.refused }));
        const refused = within(getByRole('table'));
        expect(refused.getByText('garage:giveVehicle')).not.toBeNull();
        expect(refused.getByText('garage')).not.toBeNull();
    });

    it('should total the state bag writes by key, with what goes out to clients', async () => {
        const bags: DebugBags = {
            status: COUNTING,
            seconds: 300,
            totals: { writes: 12_600, bytes: 210_000, replicatedBytes: 201_600, keys: 3, resources: 2 },
            keys: [
                {
                    key: 'fuel',
                    kind: 'entity',
                    resources: ['garage'],
                    resourceCount: 1,
                    count: 1200,
                    bytes: 372_000,
                    largest: 2100,
                    replicated: 1200,
                },
                {
                    key: 'weather',
                    kind: 'global',
                    resources: ['garage'],
                    resourceCount: 1,
                    count: 300,
                    bytes: 19_200,
                    largest: 64,
                    replicated: 0,
                },
                {
                    key: 'proximity',
                    kind: 'player',
                    resources: ['pma-voice'],
                    resourceCount: 1,
                    count: 8400,
                    bytes: 75_600,
                    largest: 9,
                    replicated: 4200,
                },
            ],
            resources: [
                { resource: 'garage', count: 1500, bytes: 391_200, replicatedBytes: 372_000, keys: 2 },
            ],
            largest: [{ bag: 'entity:1042', key: 'fuel', resource: 'garage', bytes: 2100, replicated: true }],
            truncated: false,
        };
        serve({ 'GET /debug/bags': () => answer(bags) });
        const { findByRole, getByRole, getByText } = open(<DebugBagsPage />);

        const rows = within(await findByRole('table'))
            .getAllByRole('row')
            .slice(1);
        expect(cells(rows[0])).toEqual([
            'fuel',
            words.bags.kinds.entity,
            'garage',
            '4',
            words.sizes('310 bytes', '2.1 KB'),
            words.bags.every,
        ]);
        expect(cells(rows[1])[5]).toBe(words.bags.staysHere);
        expect(cells(rows[2])[5]).toBe(words.bags.some('50%'));
        expect(getByText(words.bags.totals.sentShare('96%'))).not.toBeNull();
        expect(getByText(words.bags.largestOf('entity:1042', 'fuel'))).not.toBeNull();
        fireEvent.click(getByRole('button', { name: words.bags.views.resources }));
        expect(within(getByRole('table')).getByText('garage')).not.toBeNull();
    });

    it('should mark the hosts never called before, and stop once the owner knows them', async () => {
        const http = words.http;
        let known = false;
        const requests = (): DebugHttp => ({
            status: COUNTING,
            seconds: 900,
            totals: { requests: 630, bytes: 270_000, hosts: 2, resources: 2, newHosts: known ? 0 : 1 },
            requests: [
                {
                    host: 'paste.example',
                    method: 'POST',
                    resource: 'garage',
                    count: 15,
                    bytes: 34_500,
                    firstCalled: '2026-11-10T20:40:00Z',
                    new: !known,
                },
                {
                    host: 'discord.com',
                    method: 'POST',
                    resource: 'logs',
                    count: 540,
                    bytes: 216_000,
                    firstCalled: '2026-11-09T10:00:00Z',
                    new: false,
                },
            ],
            learningUntil: '2026-11-10T10:00:00Z',
            truncated: false,
        });
        const asked = serve({
            'GET /debug/http': () => answer(requests()),
            'POST /debug/http/known': () => {
                known = true;
                return new Response(null, { status: 204 });
            },
        });
        const { findByRole, getByRole, getByText, queryByText } = open(<DebugHttpPage />);

        const table = within(await findByRole('table'));
        expect(table.getAllByRole('row')).toHaveLength(3);
        // 540 in a quarter of an hour is 36 a minute, each with 400 bytes.
        expect(table.getByText('36')).not.toBeNull();
        // What asks for the owner's eye comes before the list it is drawn from.
        const alert = getByRole('heading', { name: http.fresh.title(1) });
        const before = alert.compareDocumentPosition(getByRole('table')) & Node.DOCUMENT_POSITION_FOLLOWING;
        expect(before).not.toBe(0);
        fireEvent.click(getByRole('button', { name: http.views.fresh(1) }));
        expect(within(getByRole('table')).getAllByRole('row')).toHaveLength(2);
        fireEvent.click(getByRole('button', { name: http.fresh.knowNamed('paste.example') }));

        await waitFor(() => expect(queryByText(http.fresh.title(1))).toBeNull());
        expect(asked.find((call) => call.route === 'POST /debug/http/known')?.body).toEqual({
            resource: 'garage',
            host: 'paste.example',
        });
        expect(getByText(http.noneNew)).not.toBeNull();
    });

    it('should show the hosts to a reader without the button that judges them', async () => {
        serve({
            'GET /debug/http': () =>
                answer({
                    status: COUNTING,
                    seconds: 900,
                    totals: { requests: 15, bytes: 34_500, hosts: 1, resources: 1, newHosts: 1 },
                    requests: [
                        {
                            host: 'paste.example',
                            method: 'POST',
                            resource: 'garage',
                            count: 15,
                            bytes: 34_500,
                            firstCalled: '2026-11-10T20:40:00Z',
                            new: true,
                        },
                    ],
                    learningUntil: null,
                    truncated: false,
                } satisfies DebugHttp),
        });
        const { findByText, queryByRole } = open(<DebugHttpPage />, READER);

        expect(await findByText(words.http.fresh.title(1))).not.toBeNull();
        expect(queryByRole('button', { name: words.http.fresh.knowNamed('paste.example') })).toBeNull();
    });
});
