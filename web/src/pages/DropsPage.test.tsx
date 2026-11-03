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

/** The way to the settings that sat at the bottom of the page, now in Data kept. */
const settingsLink = () =>
    [...document.querySelectorAll('a')].find((link) =>
        link.textContent?.startsWith(en.settings.moved(en.settings.categories.data.name)),
    );

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

    it('should lead the owner to the retention, now on the Settings page, and nobody else', async () => {
        await open('/drops', OWNER);
        expect(settingsLink()?.getAttribute('href')).toBe('/settings/data#setting-sessionDays');
        cleanup();

        await open('/drops', MODERATOR);
        expect(settingsLink()).toBeUndefined();
    });

    it('should keep the keyboard on Apply, and a draft through a change of range', async () => {
        const { router } = await open('/drops', MODERATOR);

        const apply = card('drops-log').getByRole('button', { name: en.drops.log.apply });
        fireEvent.change(card('drops-log').getByLabelText(en.drops.log.resource), {
            target: { value: 'afk_kick' },
        });
        apply.focus();
        fireEvent.click(apply);
        await settle();
        await settle();

        expect(router.state.location.search).toEqual({ resource: 'afk_kick' });
        // The same button, never drawn anew: the keyboard stays where it was.
        expect(document.activeElement).toBe(apply);
        expect(card('drops-log').getByRole('button', { name: en.drops.log.apply })).toBe(apply);

        fireEvent.change(card('drops-log').getByLabelText(en.drops.log.player), { target: { value: 'Ru' } });
        fireEvent.click(screen.getByRole('button', { name: en.drops.ranges['30d'] }));
        await settle();
        await settle();
        expect(router.state.location.search).toEqual({ range: '30d', resource: 'afk_kick' });
        expect((card('drops-log').getByLabelText(en.drops.log.player) as HTMLInputElement).value).toBe('Ru');
    });

    it('should refuse a run that is not a number under its field, and keep the address', async () => {
        const { router } = await open('/drops', MODERATOR);

        const log = card('drops-log');
        fireEvent.change(log.getByLabelText(en.drops.log.run), { target: { value: 'forty' } });
        fireEvent.click(log.getByRole('button', { name: en.drops.log.apply }));
        await settle();

        expect(log.getByText(en.drops.log.runInvalid)).not.toBeNull();
        expect(log.getByLabelText(en.drops.log.run).getAttribute('aria-invalid')).toBe('true');
        expect(router.state.location.search).toEqual({});
    });

    it('should not announce the coverage, nor the log count but after Load more', async () => {
        await open(
            '/drops',
            MODERATOR,
            api(MODERATOR, (url) =>
                url.pathname === `${DROPS}/log` && url.searchParams.has('before')
                    ? json({ rows: [{ ...LOG.rows[0]!, id: 800, at: '2026-10-28T20:00:00Z' }], next: null })
                    : undefined,
            ),
        );

        const note = screen.getByText(en.drops.coverage(82, 1_520, '5.4%', 70, 12));
        expect(note.closest('[role]')).toBeNull();
        const log = card('drops-log');
        // The footer's count is text; its own status region says nothing until more is loaded.
        const footer = log.getByText(en.drops.log.shown(3)).parentElement!;
        expect(within(footer).getByRole('status').textContent).toBe('');

        fireEvent.click(log.getByRole('button', { name: en.drops.log.loadMore }));
        await settle();
        await settle();
        expect(within(footer).getByRole('status').textContent).toBe(en.drops.log.allShown(4));
    });

    it('should leave out a group for a viewer without identifiers', async () => {
        const { fetch } = await open('/drops?group=5&profile=7', MODERATOR);

        const log = card('drops-log');
        expect(
            log.queryByRole('button', { name: en.drops.log.removeFilter(en.drops.log.group(5)) }),
        ).toBeNull();
        expect(
            log.getByRole('button', { name: en.drops.log.removeFilter(en.drops.log.profile(7)) }),
        ).not.toBeNull();
        expect(asked(fetch, 'log')).toEqual(['?range=7d&profile=7']);
    });

    it('should let a player typed take the place of the profile kept', async () => {
        const { router } = await open('/drops?profile=7', MODERATOR);

        const log = card('drops-log');
        fireEvent.change(log.getByLabelText(en.drops.log.player), { target: { value: 'Ruby' } });
        fireEvent.click(log.getByRole('button', { name: en.drops.log.apply }));
        await settle();
        await settle();

        expect(router.state.location.search).toEqual({ player: 'Ruby' });
    });

    it('should say a refusal of the group where the group is, not under the player field', async () => {
        const refusal = { code: 'forbidden', params: { permission: 'playerIdentifiersView' } };
        await open(
            '/drops?group=5',
            OWNER,
            api(OWNER, (url) => (url.pathname === `${DROPS}/log` ? json(refusal, 403) : undefined)),
        );

        const log = card('drops-log');
        const message = en.errors.forbidden(refusal.params);
        const chip = log.getByRole('button', { name: en.drops.log.removeFilter(en.drops.log.group(5)) });
        const said = log.getByText(message);
        expect(chip.getAttribute('aria-describedby')).toBe(said.id);
        expect(log.getByLabelText(en.drops.log.player).getAttribute('aria-invalid')).toBeNull();
    });

    it('should make a category a button only where it narrows something', async () => {
        await open('/drops', ANALYST);

        const breakdown = card('drops-breakdown');
        expect(breakdown.queryByRole('button', { name: en.drops.categories.quit })).toBeNull();
        expect(breakdown.queryByRole('button', { name: en.drops.categories.serverStop })).toBeNull();
        expect(breakdown.getAllByRole('button', { name: en.drops.categories.timeout })).toHaveLength(2);
        expect(breakdown.getAllByRole('button', { name: en.drops.categories.unknown })).toHaveLength(2);
        cleanup();

        await open('/drops', MODERATOR);
        expect(
            card('drops-breakdown').getAllByRole('button', { name: en.drops.categories.quit }),
        ).toHaveLength(2);
    });

    it('should word the times of the chart in the server zone and name its baseline', async () => {
        await open(
            '/drops',
            MODERATOR,
            api(MODERATOR, (url) =>
                url.pathname === `${DROPS}/summary` ? json({ ...SUMMARY, zone: 'Asia/Tokyo' }) : undefined,
            ),
        );

        // The highest rate's hour, 20:00 UTC, is 05:00 the next day in Tokyo.
        const caption = document.querySelector('figcaption')?.textContent ?? '';
        expect(caption).toContain(en.drops.chart.peak('6.4', '31 Oct 05:00'));
        expect(caption).toContain(en.drops.chart.baselineSpan('3', '3.4'));
        expect(caption).toContain(
            en.charts.markers.inWords(en.charts.markers.crash, '31 Oct 03:40', '31 Oct 03:42'),
        );
    });

    it('should count the repeat crashers it cannot name after those it names', async () => {
        await open(
            '/drops',
            MODERATOR,
            api(MODERATOR, (url) =>
                url.pathname === `${DROPS}/summary`
                    ? json({
                          ...SUMMARY,
                          repeatCrashers: [...SUMMARY.repeatCrashers, { crashes: 4, player: null }],
                      })
                    : undefined,
            ),
        );

        const signatures = card('drops-signatures');
        expect(signatures.getByRole('link', { name: 'Ruby Vex' }).getAttribute('href')).toBe('/players/7');
        expect(signatures.getByText(/, and 1 other player\.$/)).not.toBeNull();
    });

    it('should open the hints of New and Unconfirmed from the keyboard', async () => {
        await open('/drops', MODERATOR);

        const [fresh] = card('drops-signatures').getAllByRole('button', { name: en.drops.signatures.new });
        fireEvent.click(fresh!);
        expect(screen.getByRole('tooltip').textContent).toBe(en.drops.signatures.newHint);
        fireEvent.keyDown(window, { key: 'Escape' });

        const unconfirmed = card('drops-log').getAllByRole('button', { name: en.drops.log.unconfirmed })[0]!;
        fireEvent.click(unconfirmed);
        expect(screen.getByRole('tooltip').textContent).toBe(en.drops.log.unconfirmedHint);
    });
});
