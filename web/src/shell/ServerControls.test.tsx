import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';

import type { LaunchReadiness } from '../api/generated/LaunchReadiness';
import type { ServerState } from '../api/generated/ServerState';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { ServerStatusProvider } from '../server/context';
import { ServerControls } from './ServerControls';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const status = (state: ServerState, launch: LaunchReadiness = 'ready') => ({
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
});

/** The controls of an operator who may start and stop FXServer, in `state`. */
const controls = async (state: ServerState, launch?: LaunchReadiness) => {
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
                        permissions: '9',
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
            JSON.stringify({ type: 'snapshot', topic: 'server.status', data: status(state, launch) }),
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

    afterEach(cleanup);

    it('should say why a control does nothing once it is pressed', async () => {
        const { getByRole, getByText, live } = await controls('online');

        fireEvent.click(getByRole('button', { name: en.controls.start }));

        expect(getByText(en.controls.unavailable(en.controls.start, en.controls.running))).not.toBeNull();
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
        const { getByRole, queryByText, live, state } = await controls('online');
        fireEvent.click(getByRole('button', { name: en.controls.start }));

        state('stopped');

        expect(queryByText(en.controls.unavailable(en.controls.start, en.controls.running))).toBeNull();
        live.stop();
    });

    it('should not bring back why a control did nothing when the server returns to that state', async () => {
        const { getByRole, queryByText, live, state } = await controls('online');
        fireEvent.click(getByRole('button', { name: en.controls.start }));

        state('stopped');
        state('starting');
        state('online');

        expect(queryByText(en.controls.unavailable(en.controls.start, en.controls.running))).toBeNull();
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
