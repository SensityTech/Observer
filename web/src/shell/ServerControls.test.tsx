import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { LaunchReadiness } from '../api/generated/LaunchReadiness';
import type { ServerState } from '../api/generated/ServerState';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { ServerStatusProvider } from '../server/context';
import { ServerControls } from './ServerControls';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const status = (state: ServerState, launch: LaunchReadiness = 'ready'): ServerStatus => ({
    state,
    cause: null,
    since: '2026-10-28T14:00:00Z',
    startedAt: null,
    generation: 1,
    problems: [],
    build: null,
    players: null,
    restartAt: null,
    launch,
    name: null,
    port: null,
    start: null,
    stopHolders: [],
    stop: null,
});

/** Dashboard and server control, and the dashboard alone. */
const OPERATOR = '9';
const VIEWER = '1';

/** A stop that began 14 s ago and waits 46 s more, at most, for `holding`. */
const held = (holding: string[]): Partial<ServerStatus> => ({
    stop: {
        since: new Date(Date.now() - 14_000).toISOString(),
        limitAt: new Date(Date.now() + 46_500).toISOString(),
        holding,
        released: [],
    },
});

/**
 * The controls of an operator who may start and stop FXServer, in `state`; `more` changes the
 * status they open on, `permissions` who looks at them.
 */
const controls = async (
    state: ServerState,
    launch?: LaunchReadiness,
    more: Partial<ServerStatus> = {},
    permissions = OPERATOR,
) => {
    let socket: SocketEvents | undefined;
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: () => {}, close: () => {} };
        },
        refetch: () => {},
    });
    const view = render(
        <QueryClientProvider client={new QueryClient()}>
            <I18nProvider>
                <MeContext
                    value={{
                        id: 2,
                        username: 'kenji',
                        isOwner: false,
                        permissions,
                        mustChangePassword: false,
                    }}
                >
                    <LiveContext value={live}>
                        <ServerStatusProvider>
                            <ServerControls />
                        </ServerStatusProvider>
                    </LiveContext>
                </MeContext>
            </I18nProvider>
        </QueryClientProvider>,
    );
    live.start();
    await settle();
    act(() => {
        socket?.message(JSON.stringify({ type: 'hello', connectionId: 1, contract: 1 }));
        socket?.message(
            JSON.stringify({
                type: 'snapshot',
                topic: 'server.status',
                data: { ...status(state, launch), ...more },
            }),
        );
    });
    const changeTo = (next: ServerState) =>
        act(() =>
            socket?.message(JSON.stringify({ type: 'event', topic: 'server.status', data: status(next) })),
        );
    return { ...view, live, state: changeTo };
};

describe('ServerControls', () => {
    beforeAll(() => {
        // jsdom has the element but not its modal methods.
        HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
            this.open = true;
        };
        HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
            this.open = false;
        };
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should name the resources a stop waits for and let whoever controls the server quit now', async () => {
        const fetch = vi.fn(async () => new Response(null, { status: 204 }));
        vi.stubGlobal('fetch', fetch);
        const holding = ['esx_vehicleshop', 'qb-inventory'];
        const { getByRole, getByText, queryByRole, live, state } = await controls(
            'stopping',
            'ready',
            held(holding),
        );

        expect(getByText(new RegExp(`^${en.controls.waiting(holding)} `)).textContent).toBe(
            `${en.controls.waiting(holding)} ${en.controls.quitsIn('46 s')}`,
        );
        fireEvent.click(getByRole('button', { name: en.controls.quitNow }));
        const dialog = getByRole('alertdialog', { name: en.controls.quitNowTitle });
        expect(dialog.textContent).toContain(en.controls.quitNowBody(holding));
        fireEvent.click(
            [...dialog.querySelectorAll('button')].find(
                (button) => button.textContent === en.controls.quitNow,
            ) as HTMLButtonElement,
        );

        await waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
        const [url, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
        expect([url, init.method]).toEqual(['/api/servers/default/stop/now', 'POST']);
        // Once FXServer has quit, nothing of the wait is left.
        state('stopped');
        expect(queryByRole('button', { name: en.controls.quitNow })).toBeNull();
        live.stop();
    });

    it('should show the wait without its way out to an account that cannot stop the server', async () => {
        const { getByText, queryByRole, live } = await controls(
            'stopping',
            'ready',
            held(['qb-inventory']),
            VIEWER,
        );

        expect(getByText(new RegExp(`^${en.controls.waiting(['qb-inventory'])} `))).not.toBeNull();
        expect(queryByRole('button', { name: en.controls.quitNow })).toBeNull();
        live.stop();
    });

    it('should say nothing of a wait once every resource released the stop', async () => {
        const { queryByText, queryByRole, live } = await controls('stopping', 'ready', {
            stop: {
                since: new Date(Date.now() - 2_000).toISOString(),
                limitAt: new Date(Date.now() + 58_000).toISOString(),
                holding: [],
                released: [{ resource: 'qb-inventory', afterMs: 1800 }],
            },
        });

        expect(queryByText(/release the stop/)).toBeNull();
        expect(queryByRole('button', { name: en.controls.quitNow })).toBeNull();
        live.stop();
    });

    it('should tell who holds the stop before a stop or a restart is confirmed', async () => {
        const holders = ['qb-inventory'];
        const { getByRole, getByText, live } = await controls('online', 'ready', { stopHolders: holders });

        fireEvent.click(getByRole('button', { name: en.controls.restart }));

        expect(getByRole('alertdialog', { name: en.controls.restartTitle })).not.toBeNull();
        expect(getByText(en.controls.heldBy(holders))).not.toBeNull();
        live.stop();
    });

    it('should say why the wait was not skipped', async () => {
        vi.stubGlobal(
            'fetch',
            vi.fn(async () => Response.json({ code: 'stop_not_held' }, { status: 409 })),
        );
        const { getByRole, findByRole, live } = await controls('stopping', 'ready', held(['qb-inventory']));

        fireEvent.click(getByRole('button', { name: en.controls.quitNow }));
        const dialog = getByRole('alertdialog', { name: en.controls.quitNowTitle });
        fireEvent.click(
            [...dialog.querySelectorAll('button')].find(
                (button) => button.textContent === en.controls.quitNow,
            ) as HTMLButtonElement,
        );

        expect((await findByRole('alert')).textContent).toBe(
            en.controls.failed(en.controls.quitNow, en.errors.stop_not_held),
        );
        live.stop();
    });

    it('should offer Restart while the server runs, and Start in its place while it does not', async () => {
        const { queryByRole, live, state } = await controls('online');
        const offered = () =>
            [en.controls.start, en.controls.restart, en.controls.stop].filter(
                (name) => queryByRole('button', { name }) !== null,
            );
        const running = offered();

        state('stopped');

        expect(running).toEqual([en.controls.restart, en.controls.stop]);
        expect(offered()).toEqual([en.controls.start, en.controls.stop]);
        live.stop();
    });

    it('should say why a control does nothing once it is pressed', async () => {
        const { getByRole, getByText, live } = await controls('stopped');

        fireEvent.click(getByRole('button', { name: en.controls.stop }));

        expect(getByText(en.controls.unavailable(en.controls.stop, en.controls.notRunning))).not.toBeNull();
        live.stop();
    });

    it('should say Start waits for a build while none is selected, before and once it is pressed', async () => {
        const { getByRole, getByText, live } = await controls('stopped', 'noBuild');
        const start = getByRole('button', { name: en.controls.start });

        expect(start.getAttribute('title')).toBe(en.controls.noBuild);
        fireEvent.click(start);

        expect(getByText(en.controls.unavailable(en.controls.start, en.controls.noBuild))).not.toBeNull();
        live.stop();
    });

    it('should drop why a control did nothing once the state it was pressed in is gone', async () => {
        const { getByRole, queryByText, live, state } = await controls('stopped');
        fireEvent.click(getByRole('button', { name: en.controls.stop }));

        state('starting');

        expect(queryByText(en.controls.unavailable(en.controls.stop, en.controls.notRunning))).toBeNull();
        live.stop();
    });

    it('should not bring back why a control did nothing when the server returns to that state', async () => {
        const { getByRole, queryByText, live, state } = await controls('stopped');
        fireEvent.click(getByRole('button', { name: en.controls.stop }));

        state('starting');
        state('online');
        state('stopped');

        expect(queryByText(en.controls.unavailable(en.controls.stop, en.controls.notRunning))).toBeNull();
        live.stop();
    });

    it('should offer to keep a crashed server stopped rather than warn of a shutdown', async () => {
        const { getByRole, queryByText, live } = await controls('crashed');

        fireEvent.click(getByRole('button', { name: en.controls.stop }));

        expect(getByRole('alertdialog', { name: en.controls.keepStoppedTitle })).not.toBeNull();
        expect(queryByText(en.controls.noticeFirst)).toBeNull();
        live.stop();
    });
});
