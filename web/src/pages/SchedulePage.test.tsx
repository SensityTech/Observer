import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ScheduleStatus } from '../api/generated/ScheduleStatus';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { ScheduleProvider } from '../schedule/context';
import { ServerStatusProvider } from '../server/context';
import { StatusBar } from '../shell/StatusBar';
import { SchedulePage } from './SchedulePage';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

// Wednesday 28 October 2026, 23:00 in Paris: the next 04:00 there is on Thursday 29.
const NOW = Date.parse('2026-10-28T22:00:00Z');

const SCHEDULE: ScheduleStatus = {
    settings: {
        enabled: true,
        mode: 'fixedTimes',
        times: ['04:00', '16:00'],
        intervalMinutes: 360,
        timeZone: 'Europe/Paris',
        warnings: [30, 15, 10, 5, 4, 3, 2, 1],
        skipAfterStart: 120,
    },
    version: 4,
    zone: 'Europe/Paris',
    next: { at: '2026-10-29T03:00:00Z', source: 'schedule', by: null, delayedFrom: null },
    upcoming: [
        { at: '2026-10-29T03:00:00Z', time: '04:00', skipped: null, delayed: null },
        {
            at: '2026-10-29T15:00:00Z',
            time: '16:00',
            skipped: { kind: 'manual', by: 'grace' },
            delayed: null,
        },
    ],
    oneOff: null,
};

const ONLINE = {
    state: 'online',
    cause: { kind: 'ready' },
    since: '2026-10-28T20:00:00Z',
    startedAt: '2026-10-28T20:00:00Z',
    generation: 1,
    problems: [],
    build: 35245,
    players: null,
    restartAt: null,
    launch: 'ready',
};

/** The page, and the status bar, for an account holding `permissions`, the schedule as the topic says. */
const page = async (permissions: string, schedule: ScheduleStatus = SCHEDULE) => {
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
        <QueryClientProvider client={new QueryClient({ defaultOptions: { mutations: { retry: false } } })}>
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
                            <ScheduleProvider>
                                <StatusBar />
                                <SchedulePage />
                            </ScheduleProvider>
                        </ServerStatusProvider>
                    </LiveContext>
                </MeContext>
            </I18nProvider>
        </QueryClientProvider>,
    );
    live.start();
    await settle();
    const say = (message: unknown) => act(() => socket?.message(JSON.stringify(message)));
    say({ type: 'hello', connectionId: 1, contract: 1 });
    say({ type: 'snapshot', topic: 'server.status', data: ONLINE });
    say({ type: 'snapshot', topic: 'server.schedule', data: schedule });
    const publish = (next: ScheduleStatus) => say({ type: 'event', topic: 'server.schedule', data: next });
    return { ...view, live, publish, say };
};

describe('SchedulePage', () => {
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
        vi.unstubAllGlobals();
    });

    it('should show every time in the schedule zone, whatever the browser zone', async () => {
        vi.useFakeTimers({ now: NOW, toFake: ['Date'] });
        const { getByText, getAllByText, live } = await page('1');

        expect(getAllByText('04:00').length).toBeGreaterThan(0);
        expect(getByText('Thu 29 Oct · in 5h')).not.toBeNull();
        expect(getByText(en.schedule.zone('Europe/Paris', 'UTC+1'))).not.toBeNull();
        expect(getByText(en.schedule.skippedBy('grace'))).not.toBeNull();
        expect(getByText('29 Oct 04:00')).not.toBeNull();
        live.stop();
    });

    it('should say why an action does nothing for an account that cannot take it', async () => {
        const { getByRole, getByText, live } = await page('1');

        fireEvent.click(getByRole('button', { name: en.schedule.skip }));
        fireEvent.click(getByRole('button', { name: en.schedule.restartOnce }));

        expect(getByRole('button', { name: en.schedule.skip }).getAttribute('aria-disabled')).toBe('true');
        expect(
            getByText(en.schedule.unavailable(en.schedule.restartOnce, en.schedule.notPermittedOnce)),
        ).not.toBeNull();
        expect(getByText(en.schedule.readOnly)).not.toBeNull();
        live.stop();
    });

    it('should count the changes typed in the save bar', async () => {
        const { getByRole, getByText, queryByText, live } = await page('11');
        expect(queryByText(en.schedule.changes(1))).toBeNull();

        fireEvent.click(getByRole('button', { name: en.schedule.removeTime('16:00') }));
        fireEvent.click(getByRole('switch', { name: en.schedule.enabled }));

        expect(getByText(en.schedule.changes(2))).not.toBeNull();
        fireEvent.click(getByRole('button', { name: en.schedule.discard }));
        expect(queryByText(en.schedule.changes(2))).toBeNull();
        live.stop();
    });

    it('should follow newer stored settings while nothing is typed, and warn once something is', async () => {
        const { getByRole, getByText, queryByText, publish, live } = await page('11');
        const newer = { ...SCHEDULE, version: 5, settings: { ...SCHEDULE.settings, times: ['05:00'] } };

        publish(newer);
        const times = within(getByRole('list', { name: en.schedule.times }));
        expect(times.queryByText('05:00')).not.toBeNull();
        expect(queryByText(en.schedule.changedElsewhere)).toBeNull();

        fireEvent.click(getByRole('button', { name: en.schedule.removeTime('05:00') }));
        publish({ ...newer, version: 6 });

        expect(getByText(en.schedule.changedElsewhere)).not.toBeNull();
        live.stop();
    });

    it('should show what the save refused next to the settings it names', async () => {
        const fetch = vi.fn(
            async () =>
                new Response(
                    JSON.stringify({
                        code: 'invalid_schedule',
                        params: { issues: [{ field: 'time', index: 0, problem: 'tooClose' }] },
                    }),
                    { status: 400, headers: { 'content-type': 'application/json' } },
                ),
        );
        vi.stubGlobal('fetch', fetch);
        // jsdom lays nothing out, so it has no scrolling to the first setting to fix.
        Element.prototype.scrollIntoView = () => {};
        const { getByRole, findByText, live } = await page('11');

        fireEvent.click(getByRole('switch', { name: en.schedule.enabled }));
        fireEvent.click(getByRole('button', { name: en.schedule.save }));

        expect(await findByText(`04:00: ${en.schedule.problems.tooClose}`)).not.toBeNull();
        expect(fetch).toHaveBeenCalledOnce();
        live.stop();
    });

    it('should say none in the status bar when no restart is ahead, and leave the cell out without the topic', async () => {
        const { container, say, live } = await page('1', { ...SCHEDULE, next: null, upcoming: [] });
        const cell = () => container.querySelector('.status-next');

        expect(cell()?.textContent).toBe(`${en.status.nextRestart}${en.status.none}`);
        say({ type: 'unsubscribed', topic: 'server.schedule', reason: 'forbidden' });
        expect(cell()).toBeNull();
        live.stop();
    });
});
