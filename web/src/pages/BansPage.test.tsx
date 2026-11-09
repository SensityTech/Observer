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

import type { Ban } from '../api/generated/Ban';
import type { BanPage } from '../api/generated/BanPage';
import type { Me } from '../api/generated/Me';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { BansPage } from './BansPage';

const BANS_PATH = '/api/servers/default/bans';
const LICENSE = 'license:00000000000000000000000000000000000000aa';

const me = (permissions: string): Me => ({
    id: 2,
    username: 'maxime',
    isOwner: false,
    permissions,
    mustChangePassword: false,
});
/** Each with what it implies, as the API sends a set. */
const VIEWER = me('404000');
const BANNER = me('c04000');
const LIFTER = me('1404000');

const KAITO: Ban = {
    code: 'K7QM-2XPA',
    player: 7,
    name: 'kaito',
    license: LICENSE,
    identifiers: 4,
    tokens: 7,
    reason: 'Aimbot, seen on stream',
    by: 'maxime',
    createdAt: '2026-11-09T20:04:00Z',
    endsAt: null,
    state: 'active',
    lifted: null,
    refused: 2,
};
const TYPED: Ban = {
    ...KAITO,
    code: 'BX6E-N3QH',
    player: null,
    name: null,
    identifiers: 0,
    tokens: 1,
    reason: 'Sells cheats on our Discord',
    endsAt: '2026-11-16T20:04:00Z',
    refused: 0,
};
const PAGE: BanPage = {
    bans: [KAITO, TYPED],
    next: null,
    counts: { active: 2, ended: 5, lifted: 1 },
    zone: 'Europe/Paris',
};

const json = (body: unknown, status = 200) => Response.json(body, { status });

const api = () =>
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        if (url.pathname === BANS_PATH && init?.method === 'POST') {
            return json({ ...TYPED, code: 'NEW1-2345' }, 201);
        }
        if (url.pathname.endsWith('/lift')) {
            return new Response(null, { status: 204 });
        }
        if (url.pathname === BANS_PATH) {
            return json(PAGE);
        }
        return json({ code: 'not_found', params: {} }, 404);
    });

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const open = async (who: Me, fetch = api()) => {
    vi.stubGlobal('fetch', fetch);
    const root = createRootRoute({ component: Outlet });
    const router = createRouter({
        routeTree: root.addChildren([
            createRoute({ getParentRoute: () => root, path: '/bans', component: BansPage }),
        ]),
        history: createMemoryHistory({ initialEntries: ['/bans'] }),
    });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
        <I18nProvider>
            <QueryClientProvider client={client}>
                <MeContext value={who}>
                    <RouterProvider router={router} />
                </MeContext>
            </QueryClientProvider>
        </I18nProvider>,
    );
    await settle();
    await settle();
    return fetch;
};

/** The table's rows: the phone cards are in the document too, CSS alone tells them apart. */
const rows = () => within(screen.getByRole('table')).getAllByRole('row').slice(1);

const sent = (fetch: ReturnType<typeof api>, method: string) =>
    fetch.mock.calls
        .filter(([, init]) => init?.method === method)
        .map(([input, init]) => ({
            path: new URL(String(input), 'http://localhost').pathname,
            body: init?.body,
        }));

describe('BansPage', () => {
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

    it('should list each ban with its player, reason, end, author and code, and nothing to change for a viewer', async () => {
        await open(VIEWER);

        const [kaito, typed] = rows();
        expect(within(kaito!).getByRole('link', { name: 'kaito' }).getAttribute('href')).toBe('/players/7');
        expect(within(kaito!).getByText(en.bans.held(4, 7))).not.toBeNull();
        expect(within(kaito!).getByText(en.bans.permanent)).not.toBeNull();
        expect(within(kaito!).getByText(en.bans.refused(2))).not.toBeNull();
        expect(within(kaito!).getByText('K7QM-2XPA')).not.toBeNull();
        expect(within(typed!).getByText(en.bans.noProfile)).not.toBeNull();
        expect(within(typed!).getByText('2026-11-16 21:04')).not.toBeNull();
        expect(screen.queryByRole('button', { name: en.bans.banIdentifiers })).toBeNull();
        expect(screen.queryByRole('button', { name: en.bans.lift })).toBeNull();
        expect(screen.getByRole('button', { name: `${en.bans.filters.ended} 5` })).not.toBeNull();
    });

    it('should ask for another filter by its name', async () => {
        const fetch = await open(VIEWER);

        fireEvent.click(screen.getByRole('button', { name: `${en.bans.filters.lifted} 1` }));
        await settle();

        const asked = fetch.mock.calls.map(([input]) => new URL(String(input), 'http://localhost').search);
        expect(asked).toContain('?filter=lifted');
    });

    it('should ban identifiers typed by hand with a reason and a length, then give the ban ID', async () => {
        const fetch = await open(BANNER);

        fireEvent.click(screen.getByRole('button', { name: en.bans.banIdentifiers }));
        const dialog = within(screen.getByRole('dialog'));
        fireEvent.click(
            dialog.getByRole('button', { name: en.bans.dialog.submit(en.bans.dialog.lengths.day) }),
        );
        expect(dialog.getByText(en.bans.dialog.reasonMissing)).not.toBeNull();
        expect(sent(fetch, 'POST')).toEqual([]);
        fireEvent.click(dialog.getByRole('button', { name: en.bans.dialog.lengths.permanent }));
        fireEvent.change(dialog.getByLabelText(en.bans.dialog.reason), { target: { value: '  Cheats  ' } });
        fireEvent.change(dialog.getByLabelText(en.bans.dialog.identifiers), {
            target: { value: `${LICENSE}\n\n discord:123456789012345678 ` },
        });
        fireEvent.click(dialog.getByRole('button', { name: en.bans.dialog.submitPermanent }));
        await settle();
        await settle();

        expect(sent(fetch, 'POST')).toEqual([
            {
                path: BANS_PATH,
                body: JSON.stringify({
                    player: null,
                    identifiers: [LICENSE, 'discord:123456789012345678'],
                    reason: 'Cheats',
                    minutes: null,
                }),
            },
        ]);
        expect(screen.getByText(en.bans.banned('NEW1-2345'))).not.toBeNull();
        expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('should lift an active ban with the reason given', async () => {
        const fetch = await open(LIFTER);

        fireEvent.click(within(rows()[0]!).getByRole('button', { name: en.bans.lift }));
        const dialog = within(screen.getByRole('dialog'));
        expect(dialog.getByText(en.bans.liftDialog.title('kaito'))).not.toBeNull();
        fireEvent.change(dialog.getByRole('textbox'), { target: { value: ' appeal accepted ' } });
        fireEvent.click(dialog.getByRole('button', { name: en.bans.liftDialog.submit }));
        await settle();
        await settle();

        expect(sent(fetch, 'POST')).toEqual([
            { path: `${BANS_PATH}/K7QM-2XPA/lift`, body: JSON.stringify({ reason: 'appeal accepted' }) },
        ]);
        expect(screen.getByText(en.bans.liftedNotice('K7QM-2XPA'))).not.toBeNull();
    });
});
