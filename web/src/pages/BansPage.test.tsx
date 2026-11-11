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
import type { BanTemplate } from '../api/generated/BanTemplate';
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
const EDITOR = me('20404000');
const EDITOR_AND_LIFTER = me('21404000');

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
    edits: [],
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

const TEMPLATES: BanTemplate[] = [
    { reason: 'Cheating', minutes: null },
    { reason: 'Insults in voice chat', minutes: 3 * 24 * 60 },
    { reason: 'Combat logging', minutes: 90 },
];

const json = (body: unknown, status = 200) => Response.json(body, { status });

const api = ({ page = PAGE, templates = [] as BanTemplate[] } = {}) =>
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        if (url.pathname === BANS_PATH && init?.method === 'POST') {
            return json({ ...TYPED, code: 'NEW1-2345' }, 201);
        }
        if (url.pathname.endsWith('/lift')) {
            return new Response(null, { status: 204 });
        }
        if (init?.method === 'PATCH') {
            return json(KAITO);
        }
        if (url.pathname === `${BANS_PATH}/templates`) {
            return json(templates);
        }
        if (url.pathname === BANS_PATH) {
            return json(page);
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
        // jsdom opens no popover: a row's menu stays hidden, its items are pressed where they are,
        // and closing it is nothing to do.
        HTMLElement.prototype.hidePopover ??= () => {};
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
        expect(screen.queryByRole('button', { name: en.bans.actions('K7QM-2XPA') })).toBeNull();
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

        fireEvent.click(within(rows()[0]!).getByRole('menuitem', { name: en.bans.lift, hidden: true }));
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

    it('should fill in the reason and the length of the template picked, which can still be changed', async () => {
        const fetch = await open(BANNER, api({ templates: TEMPLATES }));
        const words = en.bans.dialog;

        fireEvent.click(screen.getByRole('button', { name: en.bans.banIdentifiers }));
        await settle();
        const dialog = within(screen.getByRole('dialog'));
        const template = dialog.getByLabelText<HTMLSelectElement>(words.template);
        const reason = dialog.getByLabelText<HTMLTextAreaElement>(words.reason);
        expect(
            within(template).getByText(words.templateOption('Cheating', en.bans.permanent)),
        ).not.toBeNull();
        fireEvent.change(template, { target: { value: '1' } });

        expect(reason.value).toBe('Insults in voice chat');
        expect(
            dialog.getByRole('button', { name: words.lengths.threeDays }).getAttribute('aria-pressed'),
        ).toBe('true');
        expect(template.value).toBe('1');
        fireEvent.change(template, { target: { value: '2' } });
        expect(dialog.getByLabelText<HTMLInputElement>(words.amount).value).toBe('90');
        expect(dialog.getByLabelText<HTMLSelectElement>(words.unit).value).toBe('minutes');
        fireEvent.change(reason, { target: { value: 'Combat logging, third time' } });
        expect(template.value).toBe('');
        fireEvent.change(dialog.getByLabelText(words.identifiers), { target: { value: LICENSE } });
        fireEvent.click(dialog.getByRole('button', { name: words.submit('90 minutes') }));
        await settle();
        await settle();

        expect(sent(fetch, 'POST')).toEqual([
            {
                path: BANS_PATH,
                body: JSON.stringify({
                    player: null,
                    identifiers: [LICENSE],
                    reason: 'Combat logging, third time',
                    minutes: 90,
                }),
            },
        ]);
    });

    it('should offer no template where the settings hold none', async () => {
        await open(BANNER);

        fireEvent.click(screen.getByRole('button', { name: en.bans.banIdentifiers }));
        await settle();

        expect(within(screen.getByRole('dialog')).queryByLabelText(en.bans.dialog.template)).toBeNull();
    });

    it('should change the reason and the end of an active ban, a new length counted from now', async () => {
        const fetch = await open(EDITOR_AND_LIFTER);
        const words = en.bans.editDialog;

        fireEvent.click(within(rows()[0]!).getByRole('menuitem', { name: en.bans.edit, hidden: true }));
        const dialog = within(screen.getByRole('dialog'));
        expect(dialog.getByText(words.title('kaito'))).not.toBeNull();
        expect(dialog.getByText(words.stillNever)).not.toBeNull();
        fireEvent.click(dialog.getByRole('button', { name: words.submit }));
        expect(dialog.getByText(words.unchanged)).not.toBeNull();
        expect(sent(fetch, 'PATCH')).toEqual([]);
        fireEvent.change(dialog.getByLabelText(en.bans.dialog.reason), {
            target: { value: ' Aimbot, two clips ' },
        });
        fireEvent.click(dialog.getByRole('button', { name: en.bans.dialog.lengths.week }));
        fireEvent.click(dialog.getByRole('button', { name: words.submit }));
        await settle();
        await settle();

        expect(sent(fetch, 'PATCH')).toEqual([
            {
                path: `${BANS_PATH}/K7QM-2XPA`,
                body: JSON.stringify({
                    reason: 'Aimbot, two clips',
                    ends: { kind: 'in', minutes: 7 * 24 * 60 },
                }),
            },
        ]);
        expect(screen.getByText(en.bans.editedNotice('K7QM-2XPA'))).not.toBeNull();
        expect(screen.queryByRole('dialog')).toBeNull();
    });

    it('should send only what an edit changes', async () => {
        const fetch = await open(EDITOR_AND_LIFTER);

        fireEvent.click(within(rows()[1]!).getByRole('menuitem', { name: en.bans.edit, hidden: true }));
        const dialog = within(screen.getByRole('dialog'));
        fireEvent.click(dialog.getByRole('button', { name: en.bans.dialog.lengths.permanent }));
        fireEvent.click(dialog.getByRole('button', { name: en.bans.editDialog.submit }));
        await settle();
        await settle();

        expect(sent(fetch, 'PATCH')).toEqual([
            {
                path: `${BANS_PATH}/BX6E-N3QH`,
                body: JSON.stringify({ reason: null, ends: { kind: 'never' } }),
            },
        ]);
    });

    it('should make a permanent ban end, for who edits bans without lifting them', async () => {
        const fetch = await open(EDITOR);

        expect(within(rows()[0]!).queryByRole('menuitem', { name: en.bans.lift, hidden: true })).toBeNull();
        fireEvent.click(within(rows()[0]!).getByRole('menuitem', { name: en.bans.edit, hidden: true }));
        const dialog = within(screen.getByRole('dialog'));
        fireEvent.click(dialog.getByRole('button', { name: en.bans.dialog.lengths.day }));
        fireEvent.click(dialog.getByRole('button', { name: en.bans.editDialog.submit }));
        await settle();
        await settle();

        expect(sent(fetch, 'PATCH')).toEqual([
            {
                path: `${BANS_PATH}/K7QM-2XPA`,
                body: JSON.stringify({ reason: null, ends: { kind: 'in', minutes: 24 * 60 } }),
            },
        ]);
    });

    it('should offer no edit to who bans or lifts without the permission to edit', async () => {
        await open(BANNER);
        expect(screen.queryByRole('button', { name: en.bans.actions('K7QM-2XPA') })).toBeNull();
        cleanup();

        await open(LIFTER);
        expect(within(rows()[0]!).getByRole('menuitem', { name: en.bans.lift, hidden: true })).not.toBeNull();
        expect(within(rows()[0]!).queryByRole('menuitem', { name: en.bans.edit, hidden: true })).toBeNull();
    });

    it('should say under its reason what was changed on a ban, by whom and when', async () => {
        const edited: Ban = {
            ...KAITO,
            edits: [
                {
                    at: '2026-11-10T13:02:00Z',
                    by: 'sarah',
                    reason: { before: 'Aimbot', after: KAITO.reason },
                    ends: null,
                },
                {
                    at: '2026-11-10T08:00:00Z',
                    by: 'maxime',
                    reason: null,
                    ends: { before: '2026-11-12T20:04:00Z', after: null },
                },
            ],
        };
        await open(VIEWER, api({ page: { ...PAGE, bans: [edited, TYPED] } }));
        const words = en.bans.edits;

        const [kaito, typed] = rows();
        expect(within(kaito!).getByText(words.summary(2, 'sarah'))).not.toBeNull();
        expect(within(kaito!).getByText(words.by('sarah', '2026-11-10 14:02'))).not.toBeNull();
        expect(within(kaito!).getByText(words.reason('Aimbot'))).not.toBeNull();
        expect(within(kaito!).getByText(words.ends('2026-11-12 21:04', words.forGood))).not.toBeNull();
        expect(within(typed!).queryByText(/^Edited/)).toBeNull();
    });
});
