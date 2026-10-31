import { createMemoryHistory, createRootRoute, createRouter, RouterProvider } from '@tanstack/react-router';
import { act, cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ChangesLive } from '../api/generated/ChangesLive';
import { BASELINE, LATEST, RESTART } from '../changes/fixtures';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { ChangesFact } from './ChangesFact';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** The fact in the state card, `server.changes` as the test says it. */
const fact = async () => {
    let socket: SocketEvents | undefined;
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: () => {}, close: () => {} };
        },
        refetch: () => {},
    });
    const router = createRouter({
        routeTree: createRootRoute({ component: ChangesFact }),
        history: createMemoryHistory({ initialEntries: ['/'] }),
    });
    const view = render(
        <I18nProvider>
            <LiveContext value={live}>
                <RouterProvider router={router} />
            </LiveContext>
        </I18nProvider>,
    );
    live.start();
    await settle();
    const say = (type: 'snapshot' | 'event', data: ChangesLive) =>
        act(() => socket?.message(JSON.stringify({ type, topic: 'server.changes', data })));
    act(() => socket?.message(JSON.stringify({ type: 'hello', connectionId: 1, contract: 1 })));
    return { ...view, live, say };
};

describe('ChangesFact', () => {
    beforeEach(() => {
        vi.stubGlobal('scrollTo', () => {});
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should say what the latest start changed and lead to the Changes page', async () => {
        const { say, getByRole, live } = await fact();
        say('snapshot', { latest: LATEST, check: null });

        const link = getByRole('link');
        expect(link.textContent).toBe(en.changes.overview.since('6 resources · build · cfg'));
        expect(link.getAttribute('href')).toBe('/changes');
        live.stop();
    });

    it('should say so when nothing changed, and while the baseline is taken', async () => {
        const { say, getByRole, live } = await fact();
        say('snapshot', { latest: RESTART, check: null });
        expect(getByRole('link').textContent).toBe(en.changes.overview.since('1 resource'));

        say('event', {
            latest: { ...RESTART, counts: RESTART.counts && { ...RESTART.counts, updated: 0 } },
            check: null,
        });
        expect(getByRole('link').textContent).toBe(en.changes.overview.none);

        say('event', { latest: { ...BASELINE, state: 'hashing' }, check: null });
        expect(getByRole('link').textContent).toBe(en.changes.overview.baseline);
        live.stop();
    });

    it('should say nothing before the first start', async () => {
        const { say, queryByRole, live } = await fact();
        say('snapshot', { latest: null, check: null });

        expect(queryByRole('link')).toBeNull();
        live.stop();
    });
});
