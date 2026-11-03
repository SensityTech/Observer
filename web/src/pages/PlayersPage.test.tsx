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

/** The way to the settings that sat at the bottom of the page, now in Data kept. */
const settingsLink = () =>
    [...document.querySelectorAll('a')].find((link) =>
        link.textContent?.startsWith(en.settings.moved(en.settings.categories.data.name)),
    );

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

    it('should name a confirmed player before their profile is written, and one left without a name', async () => {
        const confirmed = ONLINE.players[0]!;
        await open(
            '/players',
            MODERATOR,
            api((url) =>
                url.pathname === `${PLAYERS_PATH}/online`
                    ? json({
                          players: [
                              { ...confirmed, netId: 4, player: null, name: 'Lena', totalMs: 0 },
                              { ...confirmed, netId: 5, player: 9, name: '' },
                          ],
                      })
                    : undefined,
            ),
        );

        const [first, nameless] = rows('players-online');
        expect(within(first!).getByText('Lena')).not.toBeNull();
        expect(within(first!).queryByRole('link')).toBeNull();
        expect(within(first!).queryByText(en.players.now.unconfirmed)).toBeNull();
        expect(within(nameless!).getByRole('link', { name: en.players.noName }).getAttribute('href')).toBe(
            '/players/9',
        );
    });

    it('should name a labelled player by their label, with its ID, who gave it and their FiveM name', async () => {
        const ruby = ONLINE.players[0]!;
        await open(
            '/players',
            MODERATOR,
            api((url) =>
                url.pathname === `${PLAYERS_PATH}/online`
                    ? json({
                          players: [
                              {
                                  ...ruby,
                                  label: { name: 'Mara Quinn', id: 'CHAR-1234', resource: 'characters' },
                              },
                              {
                                  ...ruby,
                                  netId: 4,
                                  player: 9,
                                  name: 'Lena',
                                  label: { name: null, id: 'CHAR-9', resource: null },
                              },
                              {
                                  ...ruby,
                                  netId: 5,
                                  player: 10,
                                  name: 'Kenji',
                                  label: { name: 'Sol Reyes', id: null, resource: null },
                              },
                          ],
                      })
                    : undefined,
            ),
        );

        const [labelled, idOnly, nameOnly] = rows('players-online');
        expect(within(labelled!).getByRole('link', { name: 'Mara Quinn' }).getAttribute('href')).toBe(
            '/players/7',
        );
        expect(within(labelled!).getByText(en.players.label.fivem('Ruby Vex'))).not.toBeNull();
        const chip = within(labelled!).getByRole('button', { name: 'CHAR-1234' });
        const tip = document.getElementById(chip.getAttribute('aria-describedby')!);
        expect(tip?.textContent).toBe(en.players.label.given('characters'));
        expect(within(idOnly!).getByRole('link', { name: 'Lena' })).not.toBeNull();
        expect(within(idOnly!).getByRole('button', { name: 'CHAR-9' })).not.toBeNull();
        expect(within(idOnly!).queryByText(en.players.label.fivem('Lena'))).toBeNull();
        expect(within(nameOnly!).getByRole('link', { name: 'Sol Reyes' })).not.toBeNull();
        expect(within(nameOnly!).getByRole('button', { name: en.players.label.mark })).not.toBeNull();
        expect(within(nameOnly!).getByText(en.players.label.fivem('Kenji'))).not.toBeNull();
    });

    it('should show the last label of a player in the list, online or not', async () => {
        await open(
            '/players',
            MODERATOR,
            api((url) =>
                url.pathname === PLAYERS_PATH
                    ? json({
                          ...PLAYERS,
                          players: [
                              {
                                  ...PLAYERS.players[0]!,
                                  online: false,
                                  label: { name: 'Mara Quinn', id: 'CHAR-1234', resource: 'characters' },
                              },
                          ],
                      })
                    : undefined,
            ),
        );

        const [ruby] = rows('players-list');
        expect(within(ruby!).getByRole('link', { name: 'Mara Quinn' }).getAttribute('href')).toBe(
            '/players/7',
        );
        expect(within(ruby!).getByRole('button', { name: 'CHAR-1234' })).not.toBeNull();
        expect(within(ruby!).getByText(en.players.label.fivem('Ruby Vex'))).not.toBeNull();
    });

    it('should count the time txAdmin saw and the session under way into the total', async () => {
        await open('/players', MODERATOR);

        // 34h seen, 280h from txAdmin, and the 2h 14m of the session under way.
        const [ruby] = rows('players-list');
        expect(within(ruby!).getByText('316h')).not.toBeNull();
        expect(within(ruby!).getByText(en.players.list.imported('280h'))).not.toBeNull();
        expect(within(ruby!).getByText('23h 14m')).not.toBeNull();
        expect(within(ruby!).getByText(en.players.online)).not.toBeNull();
    });

    it('should say a player has no name rather than show an empty link', async () => {
        await open(
            '/players',
            MODERATOR,
            api((url) =>
                url.pathname === PLAYERS_PATH
                    ? json({ ...PLAYERS, players: [{ ...PLAYERS.players[0]!, name: '' }] })
                    : undefined,
            ),
        );

        const [nameless] = rows('players-list');
        expect(within(nameless!).getByRole('link', { name: en.players.noName }).getAttribute('href')).toBe(
            '/players/7',
        );
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

    it('should lead the owner alone to the retention and the import, now on the Settings page', async () => {
        await open('/players', OWNER);
        expect(settingsLink()?.getAttribute('href')).toBe('/settings/data#setting-linkDays');
        cleanup();

        const { fetch } = await open('/players', MODERATOR);
        expect(settingsLink()).toBeUndefined();
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
        expect(screen.queryByRole('heading', { name: en.players.profile.labels.title })).toBeNull();
        // Each drop shows twice: in the table, and as a card at phone width.
        expect(screen.getAllByText(en.players.profile.drops.categories.crash)).toHaveLength(2);
        expect(screen.getAllByText('afk_kick')).toHaveLength(2);
    });

    it("should list the labels the server's resources gave, the latest beside the player's facts", async () => {
        const labels = [
            {
                name: 'Mara Quinn',
                id: 'CHAR-1234',
                resource: 'characters',
                firstSeen: '2026-10-28T19:00:00Z',
                lastSeen: '2026-10-30T18:03:00Z',
            },
            {
                name: null,
                id: 'CHAR-9',
                resource: null,
                firstSeen: '2026-10-21T18:05:00Z',
                lastSeen: '2026-10-21T18:05:00Z',
            },
        ];
        await open(
            '/players/7',
            MODERATOR,
            api((url) => (url.pathname === `${PLAYERS_PATH}/7` ? json({ ...PROFILE, labels }) : undefined)),
        );

        expect(screen.getByRole('heading', { level: 1, name: 'Ruby Vex' })).not.toBeNull();
        const facts = screen.getByRole('region', { name: en.players.profile.facts });
        expect(within(facts).getByText('Mara Quinn')).not.toBeNull();
        expect(within(facts).getByRole('button', { name: 'CHAR-1234' })).not.toBeNull();
        const [latest, older] = rows('player-labels');
        expect(within(latest!).getByText('Mara Quinn')).not.toBeNull();
        expect(within(latest!).getByText(en.players.profile.labels.by('characters'))).not.toBeNull();
        expect(within(latest!).getByText('2026-10-30 19:03')).not.toBeNull();
        expect(within(older!).getByText('CHAR-9')).not.toBeNull();
    });

    it('should open the drop log on this player from their disconnections', async () => {
        await open('/players/7', MODERATOR);

        const drops = card('player-drops');
        expect(drops.getByRole('link', { name: en.drops.profileLog }).getAttribute('href')).toBe(
            '/drops?range=90d&profile=7',
        );
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
        const token = `3:${'c4'.repeat(32)}`;
        expect(
            identifiers.getAllByRole('button', {
                name: en.players.profile.identifiers.copy(en.players.profile.identifiers.kinds.token, token),
            }),
        ).toHaveLength(2);
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
        // Said once: a reload or a step back and forth does not say it again.
        expect(router.state.location.state.forgotten).toBe(false);
    });

    it('should say a player has no name in the title and the forget dialog', async () => {
        await open(
            '/players/7',
            OWNER,
            api((url) => (url.pathname === `${PLAYERS_PATH}/7` ? json({ ...PROFILE, name: '' }) : undefined)),
        );

        expect(screen.getByRole('heading', { level: 1, name: en.players.noName })).not.toBeNull();
        fireEvent.click(screen.getByRole('button', { name: en.players.profile.forget.button }));
        expect(
            within(screen.getByRole('alertdialog')).getByRole('heading', {
                name: en.players.profile.forget.title(''),
            }),
        ).not.toBeNull();
    });
});
