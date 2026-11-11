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

import { AdmissionProvider } from '../admission/context';
import type { AllowlistRequest } from '../api/generated/AllowlistRequest';
import type { AllowlistStatus } from '../api/generated/AllowlistStatus';
import type { Approval } from '../api/generated/Approval';
import type { ApprovedPage } from '../api/generated/ApprovedPage';
import type { Me } from '../api/generated/Me';
import type { RequestPage } from '../api/generated/RequestPage';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { AllowlistPage } from './AllowlistPage';

const PATH = '/api/servers/default/allowlist';
const words = en.allowlist;
/** 21:42 in Paris, the zone the pages give. */
const NOW = Date.parse('2026-11-11T20:42:00Z');

const me = (permissions: string): Me => ({
    id: 2,
    username: 'maxime',
    isOwner: false,
    permissions,
    mustChangePassword: false,
});
/** Each with what it implies, as the API sends a set. */
const VIEWER = me('4000');
const MANAGER = me('8004000');
const SETTINGS_READER = me('4200');

const KAITO: AllowlistRequest = {
    code: '7KQ-2XP',
    license: `license:${'a'.repeat(40)}`,
    name: 'kaito',
    firstAt: '2026-11-11T20:14:00Z',
    lastAt: '2026-11-11T20:40:00Z',
    tries: 4,
};
const NAMELESS: AllowlistRequest = {
    code: 'M3T-9RB',
    license: `license:${'b'.repeat(40)}`,
    name: null,
    firstAt: '2026-11-09T17:05:00Z',
    lastAt: '2026-11-10T08:30:00Z',
    tries: 1,
};
const REQUESTS: RequestPage = { requests: [KAITO, NAMELESS], next: null, zone: 'Europe/Paris' };

const LENA: Approval = {
    id: 41,
    license: `license:${'c'.repeat(40)}`,
    player: 12,
    name: 'Lena',
    note: null,
    by: 'maxime',
    approvedAt: '2026-11-10T19:02:00Z',
    request: 'K2P-7VD',
};
const AHEAD: Approval = {
    id: 40,
    license: `license:${'d'.repeat(40)}`,
    player: null,
    name: null,
    note: 'Cousin of Lena',
    by: 'ada',
    approvedAt: '2026-11-08T10:00:00Z',
    request: null,
};
const APPROVED: ApprovedPage = { approved: [LENA, AHEAD], next: 'cursor', zone: 'Europe/Paris' };

const ON: AllowlistStatus = { enabled: true, waiting: 2, approved: 41 };
const NO_MAINTENANCE = { on: false, since: null, by: null, passes: 0 };

const json = (body: unknown, status = 200) => Response.json(body, { status });

const api = () =>
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const method = init?.method ?? 'GET';
        if (url.searchParams.get('search') === 'ip:10.0.0.1') {
            return json({ code: 'address_search', params: {} }, 400);
        }
        if (method === 'GET' && url.pathname === `${PATH}/requests`) {
            return json(REQUESTS);
        }
        if (method === 'GET' && url.pathname === `${PATH}/approved`) {
            return json(APPROVED);
        }
        if (url.pathname === `${PATH}/requests/deny`) {
            return json({ denied: 2 });
        }
        if (method === 'POST' && url.pathname === `${PATH}/approved`) {
            return json(AHEAD, 201);
        }
        if (url.pathname.endsWith('/approve')) {
            return json(LENA);
        }
        if (method === 'DELETE') {
            return new Response(null, { status: 204 });
        }
        return json({ code: 'not_found', params: {} }, 404);
    });

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** The page for `who`, the allowlist as the admission topic says it: `null` leaves the topic silent. */
const open = async (who: Me, status: AllowlistStatus | null = ON, fetch = api()) => {
    vi.stubGlobal('fetch', fetch);
    let socket: SocketEvents | undefined;
    const live = new Realtime({
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
            createRoute({ getParentRoute: () => root, path: '/allowlist', component: AllowlistPage }),
        ]),
        history: createMemoryHistory({ initialEntries: ['/allowlist'] }),
    });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
        <I18nProvider>
            <QueryClientProvider client={client}>
                <MeContext value={who}>
                    <LiveContext value={live}>
                        <AdmissionProvider>
                            <RouterProvider router={router} />
                        </AdmissionProvider>
                    </LiveContext>
                </MeContext>
            </QueryClientProvider>
        </I18nProvider>,
    );
    live.start();
    await settle();
    const say = (message: unknown) => act(() => socket?.message(JSON.stringify(message)));
    say({ type: 'hello', connectionId: 1, contract: 1 });
    if (status !== null) {
        say({
            type: 'snapshot',
            topic: 'server.admission',
            data: { allowlist: status, maintenance: NO_MAINTENANCE },
        });
    }
    await settle();
    await settle();
    const publish = (next: AllowlistStatus) =>
        say({
            type: 'event',
            topic: 'server.admission',
            data: { allowlist: next, maintenance: NO_MAINTENANCE },
        });
    return { fetch, live, publish };
};

/** A list's rows in its table: the phone cards are in the document too, CSS alone tells them apart. */
const rows = (caption: string) =>
    within(screen.getByRole('table', { name: caption }))
        .getAllByRole('row')
        .slice(1);

const sent = (fetch: ReturnType<typeof api>, method: string) =>
    fetch.mock.calls
        .filter(([, init]) => init?.method === method)
        .map(([input, init]) => ({
            path: new URL(String(input), 'http://localhost').pathname,
            body: init?.body,
        }));

const asked = (fetch: ReturnType<typeof api>, list: 'requests' | 'approved') =>
    fetch.mock.calls
        .map(([input]) => new URL(String(input), 'http://localhost'))
        .filter((url) => url.pathname === `${PATH}/${list}`)
        .map((url) => url.search);

describe('AllowlistPage', () => {
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
        vi.spyOn(Date, 'now').mockReturnValue(NOW);
    });

    afterEach(() => {
        cleanup();
        vi.restoreAllMocks();
        vi.unstubAllGlobals();
    });

    it('should list each request with its ID and tries and each approval with who gave it, with nothing to answer for a viewer', async () => {
        const { live } = await open(VIEWER);

        const [kaito, nameless] = rows(words.requests.caption);
        expect(within(kaito!).getByText('kaito')).not.toBeNull();
        expect(within(kaito!).getByRole('button', { name: words.requests.copyCode }).textContent).toBe(
            '7KQ-2XP',
        );
        expect(within(kaito!).getByText('21:14')).not.toBeNull();
        expect(within(kaito!).getByText(words.today)).not.toBeNull();
        expect(within(kaito!).getByText(`${words.ago('2 min')} · ${words.requests.tries(4)}`)).not.toBeNull();
        expect(within(nameless!).getByText(words.requests.noName)).not.toBeNull();
        expect(within(nameless!).getByText('2026-11-09 18:05')).not.toBeNull();
        expect(within(nameless!).getByText(words.requests.tries(1))).not.toBeNull();
        const [lena, ahead] = rows(words.approved.caption);
        expect(within(lena!).getByRole('link', { name: 'Lena' }).getAttribute('href')).toBe('/players/12');
        expect(within(lena!).getByText(words.approved.fromRequest('K2P-7VD'))).not.toBeNull();
        expect(within(lena!).getByText('2026-11-10 20:02')).not.toBeNull();
        expect(within(ahead!).getByText('Cousin of Lena')).not.toBeNull();
        expect(within(ahead!).getByText(words.approved.ahead)).not.toBeNull();
        expect(screen.getByText(words.on)).not.toBeNull();
        expect(screen.getByText(words.requests.waiting(2))).not.toBeNull();
        expect(screen.getByText(words.approved.shownOf(2, 41))).not.toBeNull();
        expect(screen.queryByRole('button', { name: words.approveLicence })).toBeNull();
        expect(screen.queryByRole('button', { name: words.requests.approveOf('kaito') })).toBeNull();
        expect(screen.queryByRole('button', { name: words.requests.denyAll })).toBeNull();
        expect(screen.queryByRole('button', { name: words.approved.removeOf('Lena') })).toBeNull();
        live.stop();
    });

    it('should approve a request by its ID and say when the player gets in', async () => {
        const { fetch, live } = await open(MANAGER);

        fireEvent.click(
            within(rows(words.requests.caption)[0]!).getByRole('button', {
                name: words.requests.approveOf('kaito'),
            }),
        );
        await settle();
        await settle();

        expect(sent(fetch, 'POST')).toEqual([{ path: `${PATH}/requests/7KQ-2XP/approve`, body: undefined }]);
        expect(screen.getByText(words.approvedNotice('kaito'))).not.toBeNull();
        live.stop();
    });

    it('should deny a request without a name by its ID', async () => {
        const { fetch, live } = await open(MANAGER);

        fireEvent.click(
            within(rows(words.requests.caption)[1]!).getByRole('button', {
                name: words.requests.denyOf('M3T-9RB'),
            }),
        );
        await settle();
        await settle();

        expect(sent(fetch, 'DELETE')).toEqual([{ path: `${PATH}/requests/M3T-9RB`, body: undefined }]);
        expect(screen.getByText(words.deniedNotice('bbbbbbbb…'))).not.toBeNull();
        live.stop();
    });

    it('should deny every request shown after asking, up to the latest try the list held', async () => {
        const { fetch, live } = await open(MANAGER);

        fireEvent.click(screen.getByRole('button', { name: words.requests.denyAll }));
        const dialog = within(screen.getByRole('alertdialog'));
        expect(dialog.getByText(words.denyAllDialog.title(2))).not.toBeNull();
        expect(sent(fetch, 'POST')).toEqual([]);
        fireEvent.click(dialog.getByRole('button', { name: words.denyAllDialog.submit }));
        await settle();
        await settle();

        expect(sent(fetch, 'POST')).toEqual([
            { path: `${PATH}/requests/deny`, body: JSON.stringify({ until: KAITO.lastAt }) },
        ]);
        expect(screen.getByText(words.deniedAllNotice(2))).not.toBeNull();
        expect(screen.queryByRole('alertdialog')).toBeNull();
        live.stop();
    });

    it('should approve a licence ahead of a first visit, once it reads as one', async () => {
        const { fetch, live } = await open(MANAGER);

        fireEvent.click(screen.getByRole('button', { name: words.approveLicence }));
        const dialog = within(screen.getByRole('dialog'));
        fireEvent.change(dialog.getByLabelText(words.dialog.licence), { target: { value: 'steam:1100001' } });
        fireEvent.click(dialog.getByRole('button', { name: words.dialog.submit }));
        expect(dialog.getByText(words.dialog.licenceWrong)).not.toBeNull();
        expect(sent(fetch, 'POST')).toEqual([]);
        fireEvent.change(dialog.getByLabelText(words.dialog.licence), {
            target: { value: ` ${'D'.repeat(40)} ` },
        });
        fireEvent.change(dialog.getByLabelText(new RegExp(words.dialog.note)), {
            target: { value: ' Cousin of Lena ' },
        });
        fireEvent.click(dialog.getByRole('button', { name: words.dialog.submit }));
        await settle();
        await settle();

        expect(sent(fetch, 'POST')).toEqual([
            {
                path: `${PATH}/approved`,
                body: JSON.stringify({ license: 'D'.repeat(40), note: 'Cousin of Lena' }),
            },
        ]);
        expect(screen.getByText(words.licenceNotice)).not.toBeNull();
        expect(screen.queryByRole('dialog')).toBeNull();
        live.stop();
    });

    it('should remove an approval after saying what it does to a connected player', async () => {
        const { fetch, live } = await open(MANAGER);

        fireEvent.click(
            within(rows(words.approved.caption)[1]!).getByRole('button', {
                name: words.approved.removeOf('Cousin of Lena'),
            }),
        );
        const dialog = within(screen.getByRole('alertdialog'));
        expect(dialog.getByText(words.removeDialog.title('Cousin of Lena'))).not.toBeNull();
        expect(dialog.getByText(words.removeDialog.lead)).not.toBeNull();
        fireEvent.click(dialog.getByRole('button', { name: words.removeDialog.submit }));
        await settle();
        await settle();

        expect(sent(fetch, 'DELETE')).toEqual([{ path: `${PATH}/approved/40`, body: undefined }]);
        expect(screen.getByText(words.removedNotice('Cousin of Lena'))).not.toBeNull();
        live.stop();
    });

    it('should search both lists at once, and say under the field what a search refuses', async () => {
        const { fetch, live } = await open(MANAGER);
        const field = screen.getByLabelText(words.search);

        fireEvent.change(field, { target: { value: ' kaito ' } });
        fireEvent.click(screen.getByRole('button', { name: words.submit }));
        await settle();
        await settle();
        const filtered = screen.queryByRole('button', { name: words.requests.denyAll });
        fireEvent.change(field, { target: { value: 'ip:10.0.0.1' } });
        fireEvent.click(screen.getByRole('button', { name: words.submit }));
        await settle();
        await settle();

        expect(asked(fetch, 'requests')).toContain('?search=kaito');
        expect(asked(fetch, 'approved')).toContain('?search=kaito');
        expect(filtered).toBeNull();
        expect(field.getAttribute('aria-invalid')).toBe('true');
        expect(screen.getByText(en.errors.address_search)).not.toBeNull();
        live.stop();
    });

    it('should say that the allowlist is off, with the way to its settings for who reads them', async () => {
        const off = await open(SETTINGS_READER, { enabled: false, waiting: 0, approved: 41 });
        const link = screen.getByRole('link', { name: words.openSettings });
        const said = screen.getByText(words.off);
        off.live.stop();
        cleanup();

        const viewer = await open(VIEWER, { enabled: false, waiting: 0, approved: 41 });

        expect(link.getAttribute('href')).toBe('/settings/allowlist');
        expect(said).not.toBeNull();
        expect(screen.getByText(words.offNotice, { exact: false })).not.toBeNull();
        expect(screen.queryByRole('link', { name: words.openSettings })).toBeNull();
        viewer.live.stop();
    });

    it('should read both lists again when a refused connection moves the counts', async () => {
        const { fetch, live, publish } = await open(VIEWER);
        const before = asked(fetch, 'requests').length;

        publish({ ...ON, waiting: 3 });
        await settle();
        await settle();

        expect(asked(fetch, 'requests').length).toBe(before + 1);
        expect(screen.getByText(words.requests.waiting(3))).not.toBeNull();
        live.stop();
    });
});
