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
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Me } from '../api/generated/Me';
import type { PlayersImport } from '../api/generated/PlayersImport';
import { importQuery } from '../api/players';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import {
    COMMUNITY,
    LICENSE,
    MODERATOR,
    ONLINE,
    OWNER,
    PLAYERS,
    PROFILE,
    SETTINGS,
} from '../players/fixtures';
import { PlayerPage } from './PlayerPage';
import { PlayersPage } from './PlayersPage';

// The library draws on a canvas, which the test DOM does not have: the charts are left blank.
vi.mock('lightweight-charts', async (importOriginal) => ({
    ...(await importOriginal<typeof import('lightweight-charts')>()),
    createChart: () => {
        const series = { setData: () => {}, applyOptions: () => {} };
        return {
            addSeries: () => series,
            applyOptions: () => {},
            priceScale: () => ({ width: () => 38, applyOptions: () => {} }),
            timeScale: () => ({ fitContent: () => {} }),
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

const PLAYERS_PATH = '/api/servers/default/players';

type Route = (url: URL, init: RequestInit | undefined) => Response | undefined;

const json = (body: unknown, status = 200) => Response.json(body, { status });

/** The players routes as Observer answers them, with `route` answering first where it wants to. */
const api = (route: Route = () => undefined) =>
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const answer = route(url, init);
        if (answer !== undefined) {
            return answer;
        }
        switch (url.pathname) {
            case `${PLAYERS_PATH}/online`:
                return json(ONLINE);
            case `${PLAYERS_PATH}/community`:
                return json(COMMUNITY);
            case `${PLAYERS_PATH}/settings`:
                return json(SETTINGS);
            case `${PLAYERS_PATH}/import`:
                return json(null);
            case PLAYERS_PATH:
                return json(PLAYERS);
            case `${PLAYERS_PATH}/7`:
                return json(PROFILE);
            default:
                return json({ code: 'not_found', params: {} }, 404);
        }
    });

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** The players pages under a router of their own, for `me`, opened at `path`. */
const open = async (path: string, me: Me, fetch = api()) => {
    vi.stubGlobal('fetch', fetch);
    const root = createRootRoute({ component: Outlet });
    const router = createRouter({
        routeTree: root.addChildren([
            createRoute({ getParentRoute: () => root, path: '/players', component: PlayersPage }),
            createRoute({ getParentRoute: () => root, path: '/players/$id', component: PlayerPage }),
        ]),
        history: createMemoryHistory({ initialEntries: [path] }),
    });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
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

/** Tables and their phone cards are both in the document, as CSS alone tells them apart: the table. */
const rows = (id: string) => card(id).getAllByRole('row').slice(1);

const writing: PlayersImport = {
    state: 'writing',
    startedAt: '2026-10-30T20:00:00Z',
    finishedAt: null,
    bytes: 3_145_728,
    read: 5_000,
    written: 1_200,
    imported: 0,
    updated: 0,
    forgotten: 0,
    invalid: 0,
};

describe('PlayersPage', () => {
    beforeAll(() => {
        HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
            this.open = true;
        };
        HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
            this.open = false;
        };
    });

    beforeEach(() => {
        vi.stubGlobal('OffscreenCanvas', TestCanvas);
        // Each navigation scrolls to the top, which jsdom does not implement.
        vi.stubGlobal('scrollTo', () => {});
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should name a confirmed player and give the others their state, without licence or link', async () => {
        await open('/players', MODERATOR);

        const [ruby, confirming, unconfirmed] = rows('players-online');
        expect(within(ruby!).getByRole('link', { name: 'Ruby Vex' }).getAttribute('href')).toBe('/players/7');
        expect(within(ruby!).getByRole('button', { name: en.players.copyLicence(LICENSE) })).not.toBeNull();
        expect(within(ruby!).getByText('2h 14m')).not.toBeNull();
        expect(within(ruby!).getByText('314h')).not.toBeNull();
        for (const [row, word] of [
            [confirming, en.players.now.confirming],
            [unconfirmed, en.players.now.unconfirmed],
        ] as const) {
            expect(within(row!).getByText(word)).not.toBeNull();
            expect(within(row!).queryByRole('link')).toBeNull();
            expect(within(row!).queryByRole('button')).toBeNull();
        }
    });

    it('should count the time txAdmin saw into the total and name it apart', async () => {
        await open('/players', MODERATOR);

        const [ruby] = rows('players-list');
        expect(within(ruby!).getByText('314h')).not.toBeNull();
        expect(within(ruby!).getByText(en.players.list.imported('280h'))).not.toBeNull();
        expect(within(ruby!).getByText(en.players.online)).not.toBeNull();
    });

    it('should say under the search why a search by another identifier is refused', async () => {
        const fetch = api((url) =>
            url.pathname === PLAYERS_PATH && url.searchParams.has('search')
                ? json({ code: 'forbidden', params: { permission: 'playerIdentifiersView' } }, 403)
                : undefined,
        );
        await open('/players', MODERATOR, fetch);
        expect(screen.getByText(en.players.list.searchHint)).not.toBeNull();

        fireEvent.change(screen.getByLabelText(en.players.list.search), {
            target: { value: 'discord:123456789012345678' },
        });
        fireEvent.click(screen.getByRole('button', { name: en.players.list.submit }));
        await settle();
        await settle();

        const refusal = en.errors.forbidden({ permission: 'playerIdentifiersView' });
        expect(refusal).toContain(en.permissions.playerIdentifiersView.name);
        expect(screen.getByRole('alert').textContent).toBe(refusal);
        expect(screen.getByLabelText(en.players.list.search).getAttribute('aria-invalid')).toBe('true');
        expect(document.querySelector('section[aria-labelledby="players-list"] table')).toBeNull();
        expect(fetch).toHaveBeenCalledWith(
            `${PLAYERS_PATH}?search=discord%3A123456789012345678`,
            expect.anything(),
        );
    });

    it('should show the import while it writes, then what it took in', async () => {
        let current = writing;
        const { client } = await open(
            '/players',
            OWNER,
            api((url) => (url.pathname === `${PLAYERS_PATH}/import` ? json(current) : undefined)),
        );

        const data = card('players-data');
        expect(data.getByText(en.players.data.writing(1_200, 5_000))).not.toBeNull();
        expect(data.getByRole('progressbar', { name: en.players.data.progress }).getAttribute('value')).toBe(
            '1200',
        );
        expect(data.getByRole('button', { name: en.players.data.start }).hasAttribute('disabled')).toBe(true);

        current = {
            ...writing,
            state: 'finished',
            finishedAt: '2026-10-30T20:01:00Z',
            written: 5_000,
            imported: 4_700,
            updated: 280,
            forgotten: 12,
            invalid: 8,
        };
        await act(() => client.refetchQueries({ queryKey: importQuery.queryKey }));
        await settle();

        expect(
            data.getByText(en.players.data.finished('2026-10-30 21:01', 4_700, 280, 12, 8)),
        ).not.toBeNull();
        expect(data.queryByRole('progressbar')).toBeNull();
    });

    it('should keep the retention and the import to the owner', async () => {
        const { fetch } = await open('/players', MODERATOR);

        expect(document.querySelector('section[aria-labelledby="players-data"]')).toBeNull();
        expect(fetch).not.toHaveBeenCalledWith(`${PLAYERS_PATH}/settings`, expect.anything());
    });
});

describe('PlayerPage', () => {
    beforeAll(() => {
        HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
            this.open = true;
        };
        HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
            this.open = false;
        };
    });

    beforeEach(() => {
        vi.stubGlobal('OffscreenCanvas', TestCanvas);
        vi.stubGlobal('scrollTo', () => {});
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should show a profile without identifiers or links to a viewer who may not see them', async () => {
        await open('/players/7', MODERATOR);

        expect(screen.getByRole('heading', { level: 1, name: 'Ruby Vex' })).not.toBeNull();
        const total = screen.getByRole('region', { name: en.players.profile.total });
        expect(within(total).getByText('316h')).not.toBeNull();
        expect(within(total).getByText(en.players.profile.beforeObserver)).not.toBeNull();
        expect(within(total).getByText(en.players.profile.inProgress)).not.toBeNull();
        expect(screen.queryByRole('heading', { name: en.players.profile.identifiers.title })).toBeNull();
        expect(screen.queryByRole('heading', { name: en.players.profile.linked.title })).toBeNull();
        expect(screen.queryByRole('button', { name: en.players.profile.forget.button })).toBeNull();
        // Each drop shows twice: in the table, and as a card at phone width.
        expect(screen.getAllByText(en.players.profile.drops.categories.crash)).toHaveLength(2);
        expect(screen.getAllByText('afk_kick')).toHaveLength(2);
    });

    it('should never show an address, only how many other licences shared it', async () => {
        await open(
            '/players/7',
            OWNER,
            api((url) =>
                url.pathname === `${PLAYERS_PATH}/7`
                    ? json({
                          ...PROFILE,
                          identifiers: [
                              {
                                  kind: 'ip',
                                  value: null,
                                  firstSeen: '2026-10-21T18:00:00Z',
                                  lastSeen: '2026-10-30T18:02:00Z',
                                  times: 9,
                                  imported: false,
                                  otherLicences: 2,
                              },
                              {
                                  kind: 'token',
                                  value: `3:${'c4'.repeat(32)}`,
                                  firstSeen: '2024-03-02T19:20:00Z',
                                  lastSeen: '2024-05-01T19:20:00Z',
                                  times: 1,
                                  imported: true,
                                  otherLicences: 0,
                              },
                          ],
                          linked: [],
                      })
                    : undefined,
            ),
        );

        const identifiers = card('player-identifiers');
        expect(identifiers.getAllByText(en.players.profile.identifiers.sameAddress(2))).toHaveLength(2);
        expect(identifiers.getAllByText('3:c4c4c4c4c4c4…')).toHaveLength(2);
        expect(identifiers.getAllByText(en.players.profile.identifiers.imported)).toHaveLength(2);
        expect(card('player-linked').getByText(en.players.profile.linked.empty)).not.toBeNull();
    });

    it('should forget the player once the start of the licence is typed, then go back to the list', async () => {
        const fetch = api((url, init) =>
            init?.method === 'DELETE' && url.pathname === `${PLAYERS_PATH}/7`
                ? new Response(null, { status: 204 })
                : undefined,
        );
        const { router } = await open('/players/7', OWNER, fetch);

        fireEvent.click(screen.getByRole('button', { name: en.players.profile.forget.button }));
        const dialog = within(screen.getByRole('alertdialog'));
        const code = LICENSE.slice('license:'.length, 'license:'.length + 8);
        for (const item of en.players.profile.forget.goes) {
            expect(dialog.getByText(item)).not.toBeNull();
        }
        const submit = dialog.getByRole('button', { name: en.players.profile.forget.submit });
        expect(submit.hasAttribute('disabled')).toBe(true);

        fireEvent.change(dialog.getByLabelText(en.players.profile.forget.confirm(code)), {
            target: { value: code },
        });
        expect(submit.hasAttribute('disabled')).toBe(false);
        fireEvent.click(submit);
        await settle();
        await settle();

        expect(fetch).toHaveBeenCalledWith(
            `${PLAYERS_PATH}/7`,
            expect.objectContaining({ method: 'DELETE' }),
        );
        expect(router.state.location.pathname).toBe('/players');
        expect(screen.getByText(en.players.forgotten)).not.toBeNull();
    });
});
