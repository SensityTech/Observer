import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
    RouterProvider,
} from '@tanstack/react-router';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { AdmissionProvider } from '../admission/context';
import type { MaintenanceStatus } from '../api/generated/MaintenanceStatus';
import type { Me } from '../api/generated/Me';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { MaintenanceBanner, MaintenanceButton } from './Maintenance';

const PATH = '/api/servers/default/maintenance';
const words = en.maintenance;
/** 21:55 UTC, the zone the banner falls back to without a schedule. */
const NOW = Date.parse('2026-11-11T21:55:00Z');

const me = (permissions: string): Me => ({
    id: 2,
    username: 'maxime',
    isOwner: false,
    permissions,
    mustChangePassword: false,
});
const VIEWER = me('1');
const OPERATOR = me('9');
/** Controls the server and reads the settings. */
const OWNER_LIKE = me('209');

const OFF: MaintenanceStatus = { on: false, since: null, by: null, passes: 3 };
const ON: MaintenanceStatus = { on: true, since: '2026-11-11T21:40:00Z', by: 'maxime', passes: 3 };

const json = (body: unknown, status = 200) => Response.json(body, { status });

const api = (serverName: string | null) =>
    vi.fn(async (_: RequestInfo | URL, init?: RequestInit) =>
        init?.method === 'PUT'
            ? new Response(null, { status: 204 })
            : json({ message: 'Back at 8, {name}.\nFollow the Discord.', serverName }),
    );

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** The status card's maintenance button and the banner under it, maintenance as the topic says. */
const open = async (who: Me, maintenance: MaintenanceStatus, serverName: string | null = null) => {
    const fetch = api(serverName);
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
    const root = createRootRoute({
        component: () => (
            <>
                <MaintenanceButton />
                <MaintenanceBanner />
            </>
        ),
    });
    const router = createRouter({
        routeTree: root.addChildren([createRoute({ getParentRoute: () => root, path: '/' })]),
        history: createMemoryHistory({ initialEntries: ['/'] }),
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
    const publish = (next: MaintenanceStatus, type = 'event') =>
        say({
            type,
            topic: 'server.admission',
            data: { allowlist: { enabled: false, waiting: 0, approved: 0 }, maintenance: next },
        });
    say({ type: 'hello', connectionId: 1, contract: 1 });
    publish(maintenance, 'snapshot');
    await settle();
    return { fetch, live, publish };
};

const put = (fetch: ReturnType<typeof api>) =>
    fetch.mock.calls
        .filter(([, init]) => init?.method === 'PUT')
        .map(([input, init]) => ({
            path: new URL(String(input), 'http://localhost').pathname,
            body: init?.body,
        }));

describe('Maintenance mode in the shell', () => {
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

    it('should say what turning maintenance on does and what players will read, then turn it on', async () => {
        const { fetch, live, publish } = await open(OWNER_LIKE, OFF, 'Aperture RP | maintenance');

        fireEvent.click(screen.getByRole('button', { name: words.button }));
        await settle();
        const dialog = within(screen.getByRole('alertdialog'));
        const lead = dialog.getByText(words.dialog.lead(3));
        const read = (await dialog.findByText(words.dialog.preview)).parentElement?.querySelector(
            'p',
        )?.textContent;
        const listed = dialog.queryByText(words.dialog.listed('Aperture RP | maintenance'));
        const link = dialog.getByRole('link', { name: words.dialog.change });
        const before = put(fetch);
        fireEvent.click(dialog.getByRole('button', { name: words.dialog.submit }));
        await settle();
        publish(ON);

        expect(lead).not.toBeNull();
        expect(read).toBe(`Back at 8, ${en.settings.maintenance.example.name}.\nFollow the Discord.`);
        expect(listed).not.toBeNull();
        expect(link.getAttribute('href')).toBe('/settings/maintenance');
        expect(before).toEqual([]);
        expect(put(fetch)).toEqual([{ path: PATH, body: JSON.stringify({ on: true }) }]);
        expect(screen.queryByRole('alertdialog')).toBeNull();
        expect(screen.queryByRole('button', { name: words.button })).toBeNull();
        expect(screen.getByText(words.on)).not.toBeNull();
        live.stop();
    });

    it('should keep the way to the settings from an account that cannot read them', async () => {
        const { live } = await open(OPERATOR, { ...OFF, passes: 0 });

        fireEvent.click(screen.getByRole('button', { name: words.button }));
        await settle();
        const dialog = within(screen.getByRole('alertdialog'));
        await dialog.findByText(words.dialog.preview);

        expect(dialog.getByText(words.dialog.lead(0))).not.toBeNull();
        expect(dialog.queryByText(/server list/)).toBeNull();
        expect(dialog.queryByRole('link', { name: words.dialog.change })).toBeNull();
        live.stop();
    });

    it('should say since when the server is closed and by whom, and end it in one press', async () => {
        const { fetch, live } = await open(OPERATOR, ON);

        const banner = screen.getByText(words.on).parentElement;
        fireEvent.click(screen.getByRole('button', { name: words.end }));
        await settle();

        expect(banner?.textContent).toBe(`${words.on} ${words.since('21:40', 'maxime')} ${words.refused(3)}`);
        expect(put(fetch)).toEqual([{ path: PATH, body: JSON.stringify({ on: false }) }]);
        live.stop();
    });

    it('should show the banner without its button to an account that does not control the server', async () => {
        const { live } = await open(VIEWER, { ...ON, since: '2026-11-10T21:40:00Z', passes: 1 });

        const banner = screen.getByText(words.on).parentElement;

        expect(banner?.textContent).toBe(
            `${words.on} ${words.since('2026-11-10 21:40', 'maxime')} ${words.refused(1)}`,
        );
        expect(words.refused(1)).toBe('Players are refused; 1 person still gets in.');
        expect(screen.queryByRole('button', { name: words.end })).toBeNull();
        expect(screen.queryByRole('button', { name: words.button })).toBeNull();
        live.stop();
    });
});
