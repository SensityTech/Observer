import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
    Outlet,
    RouterProvider,
} from '@tanstack/react-router';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Me } from '../api/generated/Me';
import { MeContext } from '../auth/me';
import {
    ANALYST,
    CHANGES,
    LOG,
    MODERATOR,
    OWNER,
    RESOURCES,
    RESOURCES_STATS,
    SERIES,
    SETTINGS,
    SIGNATURES,
    SIGNATURES_STATS,
    SUMMARY,
    SUMMARY_STATS,
    UPTIME,
} from '../drops/fixtures';
import { parseDropsSearch } from '../drops/search';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { DropsPage } from './DropsPage';

// The library draws on a canvas, which the test DOM does not have: the chart is left blank.
vi.mock('lightweight-charts', async (importOriginal) => ({
    ...(await importOriginal<typeof import('lightweight-charts')>()),
    createChart: () => {
        const series = { setData: () => {}, applyOptions: () => {}, attachPrimitive: () => {} };
        return {
            addSeries: () => series,
            applyOptions: () => {},
            priceScale: () => ({ width: () => 38, applyOptions: () => {} }),
            timeScale: () => ({
                fitContent: () => {},
                timeToCoordinate: () => null,
                subscribeVisibleLogicalRangeChange: () => {},
                unsubscribeVisibleLogicalRangeChange: () => {},
            }),
            subscribeCrosshairMove: () => {},
            unsubscribeCrosshairMove: () => {},
            remove: () => {},
        };
    },
}));

/** Every colour token reads as one colour, the way the charts resolve them through a canvas. */
class TestCanvas {
    getContext() {
        return {
            fillStyle: '',
            clearRect: () => {},
            fillRect: () => {},
            getImageData: () => ({ data: [37, 99, 235, 255] }),
        };
    }
}

class TestResizeObserver {
    observe() {}
    disconnect() {}
}

const DROPS = '/api/servers/default/drops';

type Route = (url: URL, init: RequestInit | undefined) => Response | undefined;

const json = (body: unknown, status = 200) => Response.json(body, { status });

/** The drops routes as Observer answers `me`: names and reason groups only for who sees players. */
const api = (me: Me, route: Route = () => undefined) =>
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const answer = route(url, init);
        if (answer !== undefined) {
            return answer;
        }
        const players = me.permissions !== ANALYST.permissions;
        switch (url.pathname) {
            case `${DROPS}/summary`:
                return json(players ? SUMMARY : SUMMARY_STATS);
            case `${DROPS}/series`:
                return json(SERIES);
            case `${DROPS}/signatures`:
                return json(players ? SIGNATURES : SIGNATURES_STATS);
            case `${DROPS}/resources`:
                return json(players ? RESOURCES : RESOURCES_STATS);
            case `${DROPS}/changes`:
                return json(CHANGES);
            case `${DROPS}/uptime`:
                return json(UPTIME);
            case `${DROPS}/log`:
                return players ? json(LOG) : json({ code: 'forbidden', params: {} }, 403);
            case `${DROPS}/settings`:
                return json(SETTINGS);
            default:
                return json({ code: 'not_found', params: {} }, 404);
        }
    });

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** The page under a router of its own, for `me`, opened at `path`. */
const open = async (path: string, me: Me, fetch = api(me)) => {
    vi.stubGlobal('fetch', fetch);
    const root = createRootRoute({ component: Outlet });
    const router = createRouter({
        routeTree: root.addChildren([
            createRoute({
                getParentRoute: () => root,
                path: '/drops',
                validateSearch: (search: Record<string, unknown>) => parseDropsSearch(search),
                component: DropsPage,
            }),
            createRoute({ getParentRoute: () => root, path: '/players/$id', component: () => null }),
        ]),
        history: createMemoryHistory({ initialEntries: [path] }),
    });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, refetchInterval: false } } });
    render(
        <I18nProvider>
            <QueryClientProvider client={client}>
                <MeContext value={me}>
                    <RouterProvider router={router} />
                </MeContext>
            </QueryClientProvider>
        </I18nProvider>,
    );
    await settle();
    await settle();
    return { client, fetch, router };
};

const card = (id: string) => {
    const section = document.querySelector<HTMLElement>(`section[aria-labelledby="${id}"]`);
    if (section === null) {
        throw new Error(`no card ${id}`);
    }
    return within(section);
};

/** The requests made to a drops route, as their query strings. */
const asked = (fetch: ReturnType<typeof api>, route: string) =>
    fetch.mock.calls
        .map(([input]) => new URL(String(input), 'http://localhost'))
        .filter((url) => url.pathname === `${DROPS}/${route}`)
        .map((url) => url.search);

describe('DropsPage', () => {
    beforeEach(() => {
        vi.stubGlobal('OffscreenCanvas', TestCanvas);
        vi.stubGlobal('ResizeObserver', TestResizeObserver);
        // Each navigation scrolls to the top, which jsdom does not implement.
        vi.stubGlobal('scrollTo', () => {});
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should give the tiles their rates and verdicts, and one player share without the player', async () => {
        await open('/drops', MODERATOR);

        const crashes = within(screen.getByRole('region', { name: en.drops.tiles.labels.crashes }));
        expect(crashes.getByText('4.6').textContent).toBe(`4.6${en.drops.unit}`);
        expect(crashes.getByRole('button', { name: 'Rising +38%' })).not.toBeNull();
        expect(crashes.getByText(en.drops.tiles.oneIn(23))).not.toBeNull();
        expect(crashes.getByText(en.drops.tiles.onePlayerShare('41%', 'crashes'))).not.toBeNull();
        const timeouts = within(screen.getByRole('region', { name: en.drops.tiles.labels.timeouts }));
        expect(timeouts.getByRole('button', { name: 'Falling −20%' })).not.toBeNull();
        // 82 of 1,520 sessions closed by Observer: more than 5%, said once.
        expect(screen.getByText(en.drops.coverage(82, 1_520, '5.4%', 70, 12))).not.toBeNull();
    });

    it('should keep the log, the reason groups and the names from a statistics viewer', async () => {
        const { fetch } = await open('/drops', ANALYST);

        expect(document.querySelector('section[aria-labelledby="drops-log"]')).toBeNull();
        expect(asked(fetch, 'log')).toEqual([]);
        expect(
            card('drops-resources').queryByRole('button', { name: en.drops.resources.reasons }),
        ).toBeNull();
        expect(
            card('drops-resources').queryByRole('button', { name: en.drops.resources.showInLog }),
        ).toBeNull();
        expect(
            card('drops-signatures').queryByRole('button', { name: en.drops.signatures.showInLog }),
        ).toBeNull();
        expect(card('drops-signatures').getByText(en.drops.signatures.repeat(1))).not.toBeNull();
        expect(screen.queryByText('Ruby Vex')).toBeNull();
        expect(document.querySelector('section[aria-labelledby="drops-settings"]')).toBeNull();
        expect(screen.getByText(en.drops.breakdown.filterHintChart)).not.toBeNull();
    });

    it('should tell the changes in time, worded after them, and say what they cannot hold yet', async () => {
        await open('/drops', ANALYST);

        const changes = card('drops-changes');
        expect(changes.getByText(en.drops.changes.incident(38, '2.4 s'))).not.toBeNull();
        expect(changes.getByText(en.drops.changes.crash(54))).not.toBeNull();
        expect(changes.getByText(en.drops.changes.build(16500, 17000))).not.toBeNull();
        expect(changes.getByText(en.drops.changes.after('crashes', '3.1', '4.6'))).not.toBeNull();
        expect(changes.getByText(en.drops.changes.snapshots)).not.toBeNull();
        expect(changes.queryByRole('button', { name: en.drops.changes.showInLog })).toBeNull();
    });

    it('should say a hidden signature stays hidden until a second player crashes with it', async () => {
        await open('/drops', ANALYST);

        const signatures = card('drops-signatures');
        // In the table and in its phone card.
        expect(signatures.getAllByText(en.drops.signatures.hidden)).toHaveLength(2);
    });

    it('should not open an empty list of examples, and say why a statistics viewer has none', async () => {
        await open('/drops', ANALYST);

        const signatures = card('drops-signatures');
        // Only the asset crash has an example two players shared: its button, in the table and the card.
        expect(signatures.getAllByRole('button', { name: en.drops.signatures.examples })).toHaveLength(2);
        expect(signatures.getAllByText(en.drops.signatures.noSharedExample)).toHaveLength(2);
        cleanup();

        await open('/drops', MODERATOR);
        expect(
            card('drops-signatures').getAllByRole('button', { name: en.drops.signatures.examples }),
        ).toHaveLength(4);
        expect(card('drops-signatures').queryByText(en.drops.signatures.noSharedExample)).toBeNull();
    });

    it('should say since when players are counted when the sessions kept start after the range', async () => {
        const kept = '2026-10-02T22:00:00Z';
        await open(
            '/drops?range=30d',
            MODERATOR,
            api(MODERATOR, (url) => {
                const month = { range: '30d', keptFrom: kept };
                switch (url.pathname) {
                    case `${DROPS}/summary`:
                        return json({ ...SUMMARY, ...month, from: '2026-09-30T19:00:00Z' });
                    case `${DROPS}/signatures`:
                        return json({ ...SIGNATURES, ...month });
                    case `${DROPS}/resources`:
                        return json({ ...RESOURCES, ...month });
                    case `${DROPS}/uptime`:
                        return json({ ...UPTIME, ...month });
                    default:
                        return undefined;
                }
            }),
        );

        // Midnight of 3 October in Paris.
        const crashes = within(screen.getByRole('region', { name: en.drops.tiles.labels.crashes }));
        expect(crashes.getByText(en.drops.since('3 Oct'))).not.toBeNull();
        const since = en.drops.playersSince('3 Oct');
        expect(
            card('drops-signatures').getByText(`${en.drops.signatures.meta(64)} · ${since}`),
        ).not.toBeNull();
        expect(card('drops-resources').getByText(`${en.drops.resources.meta(28)} · ${since}`)).not.toBeNull();
        expect(card('drops-uptime').getByText(en.drops.uptime.since('3 Oct'))).not.toBeNull();
    });

    it('should say nothing of it when the sessions kept cover the range', async () => {
        await open('/drops', MODERATOR);

        expect(screen.queryByText(en.drops.playersCounted)).toBeNull();
        expect(screen.queryByText(new RegExp(en.drops.playersSince('')))).toBeNull();
    });

    it('should write the examples a client wrote as plain text', async () => {
        await open('/drops', MODERATOR);

        const signatures = card('drops-signatures');
        fireEvent.click(signatures.getAllByRole('button', { name: en.drops.signatures.examples })[0]!);
        const example = SIGNATURES.signatures[0]!.examples[0]!;
        expect(signatures.getAllByText(example)).not.toHaveLength(0);
        expect(document.querySelector('section[aria-labelledby="drops-signatures"] b')).toBeNull();
    });

    it('should read the log filters from the address and ask the API for them', async () => {
        const { fetch } = await open('/drops?range=24h&category=kick&kind=resource&player=Ruby', MODERATOR);

        expect(asked(fetch, 'log')).toContain('?range=24h&category=kick&kind=resource&player=Ruby');
        const log = card('drops-log');
        expect((log.getByLabelText(en.drops.log.category) as HTMLSelectElement).value).toBe('kick');
        expect((log.getByLabelText(en.drops.log.kind) as HTMLSelectElement).value).toBe('resource');
        expect((log.getByLabelText(en.drops.log.player) as HTMLInputElement).value).toBe('Ruby');
    });

    it('should put the filters applied in the address', async () => {
        const { fetch, router } = await open('/drops', MODERATOR);

        const log = card('drops-log');
        fireEvent.change(log.getByLabelText(en.drops.log.category), { target: { value: 'timeout' } });
        fireEvent.change(log.getByLabelText(en.drops.log.run), { target: { value: '41' } });
        fireEvent.click(log.getByRole('button', { name: en.drops.log.apply }));
        await settle();
        await settle();

        expect(router.state.location.search).toEqual({ category: 'timeout', run: 41 });
        expect(router.state.location.href).toBe('/drops?category=timeout&run=41');
        expect(asked(fetch, 'log')).toContain('?range=7d&category=timeout&run=41');
    });

    it('should open the log on a resource from its row, and on a run from a server crash', async () => {
        const { router } = await open('/drops', MODERATOR);

        fireEvent.click(
            card('drops-resources').getAllByRole('button', { name: en.drops.resources.showInLog })[0]!,
        );
        await settle();
        expect(router.state.location.search).toEqual({ resource: 'afk_kick' });

        fireEvent.click(
            card('drops-changes').getAllByRole('button', { name: en.drops.changes.showInLog })[1]!,
        );
        await settle();
        expect(router.state.location.search).toEqual({ run: 40 });
    });

    it('should name the player of a drop, and say when who it was is not confirmed', async () => {
        await open('/drops', MODERATOR);

        const rows = card('drops-log').getAllByRole('row').slice(1);
        expect(within(rows[0]!).getByRole('link', { name: 'Ruby Vex' }).getAttribute('href')).toBe(
            '/players/7',
        );
        expect(within(rows[1]!).getByText(en.drops.log.unconfirmed)).not.toBeNull();
        // A server crash Observer closed names nobody, and is not "unconfirmed" either.
        expect(within(rows[2]!).queryByText(en.drops.log.unconfirmed)).toBeNull();
        expect(within(rows[2]!).getByText(en.drops.log.noPlayer)).not.toBeNull();
        // A reason a resource wrote stays text.
        expect(within(rows[1]!).getByText(LOG.rows[1]!.reason!)).not.toBeNull();
    });

    it('should narrow the log to the signature of a row', async () => {
        const { fetch, router } = await open('/drops', MODERATOR);

        const [crash] = card('drops-log').getAllByRole('row').slice(1);
        fireEvent.click(
            within(crash!).getByRole('button', {
                name: en.drops.log.filterSignature('hash:GTA5_b3095.exe+1A2B3C'),
            }),
        );
        await settle();
        await settle();

        expect(router.state.location.search).toEqual({ signature: 12 });
        expect(asked(fetch, 'log')).toContain('?range=7d&signature=12');
        // The filter offers it by its text, as the signatures card lists it.
        const picked = card('drops-log').getByLabelText(en.drops.log.signature) as HTMLSelectElement;
        expect(picked.value).toBe('12');
        expect(picked.selectedOptions[0]?.textContent).toBe('hash:GTA5_b3095.exe+1A2B3C');
    });

    it('should give the server stops and crashes no player rate and no verdict', async () => {
        await open('/drops', MODERATOR);

        const breakdown = card('drops-breakdown');
        const stops = breakdown
            .getAllByRole('row')
            .find((row) => within(row).queryByRole('button', { name: en.drops.categories.serverStop }));
        expect(within(stops!).getByText(en.drops.breakdown.notPlayers)).not.toBeNull();
        expect(within(stops!).queryByRole('button', { description: /./ })).toBeNull();
    });

    it('should narrow the chart and the log to a category picked in the breakdown', async () => {
        const { fetch, router } = await open('/drops', MODERATOR);

        const breakdown = card('drops-breakdown');
        fireEvent.click(breakdown.getAllByRole('button', { name: en.drops.categories.timeout })[0]!);
        await settle();
        await settle();

        expect(router.state.location.search).toEqual({ category: 'timeout' });
        expect(asked(fetch, 'series')).toContain('?range=7d&category=timeout');
        expect(asked(fetch, 'log')).toContain('?range=7d&category=timeout');
        expect(
            breakdown.getAllByRole('button', { name: en.drops.categories.timeout, pressed: true }),
        ).not.toHaveLength(0);
    });

    it('should give the owner the retention, and nobody else', async () => {
        await open('/drops', OWNER);
        expect(card('drops-settings').getByLabelText(en.drops.settings.sessionDays)).not.toBeNull();
        cleanup();

        await open('/drops', MODERATOR);
        expect(document.querySelector('section[aria-labelledby="drops-settings"]')).toBeNull();
    });
});
