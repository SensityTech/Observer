import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

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
    const drop = () => act(() => socket?.close(1006));
    return { ...view, live, publish, say, drop };
};

describe('SchedulePage', () => {
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

    it('should say to restore the skipped restart before skipping another', async () => {
        const { getByRole, getByText, live } = await page('11');

        fireEvent.click(getByRole('button', { name: en.schedule.skip }));

        expect(
            getByText(en.schedule.unavailable(en.schedule.skip, en.schedule.oneSkip('16:00'))),
        ).not.toBeNull();
        expect(getByRole('button', { name: en.schedule.restore })).not.toBeNull();
        live.stop();
    });

    it('should say a single restart waits while FXServer stops', async () => {
        const { getByRole, getByText, say, live } = await page('9');

        say({ type: 'event', topic: 'server.status', data: { ...ONLINE, state: 'stopping' } });
        fireEvent.click(getByRole('button', { name: en.schedule.restartOnce }));

        expect(
            getByText(en.schedule.unavailable(en.schedule.restartOnce, en.schedule.stoppingOnce)),
        ).not.toBeNull();
        live.stop();
    });

    it('should name each warning in its remove button, one minute in the singular', async () => {
        const { getByRole, live } = await page('11');

        expect(getByRole('button', { name: 'Remove the warning 1 minute before' })).not.toBeNull();
        expect(getByRole('button', { name: 'Remove the warning 30 minutes before' })).not.toBeNull();
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

    it('should keep the settings typed while the live link drops and comes back', async () => {
        const { getByRole, getByText, queryByText, say, drop, live } = await page('11');
        fireEvent.click(getByRole('button', { name: en.schedule.removeTime('16:00') }));

        drop();
        const down = queryByText(en.schedule.outOfDate);
        const keptDown = queryByText(en.schedule.changes(1));
        // The client tries again after a second.
        await act(() => new Promise((resolve) => setTimeout(resolve, 1_200)));
        say({ type: 'hello', connectionId: 2, contract: 1 });
        say({ type: 'snapshot', topic: 'server.status', data: ONLINE });
        say({ type: 'snapshot', topic: 'server.schedule', data: SCHEDULE });

        expect(down).not.toBeNull();
        expect(keptDown).not.toBeNull();
        expect(getByText(en.schedule.changes(1))).not.toBeNull();
        expect(queryByText(en.schedule.outOfDate)).toBeNull();
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

    it('should say what a number or a time needs by its field rather than send what Observer refuses', async () => {
        const fetch = vi.fn();
        vi.stubGlobal('fetch', fetch);
        Element.prototype.scrollIntoView = () => {};
        const { getByRole, getByLabelText, getByText, live } = await page('19');

        fireEvent.change(getByLabelText(en.schedule.skipAfterStart), { target: { value: '1.5' } });
        fireEvent.click(getByRole('button', { name: en.schedule.save }));
        const skipError = getByText(en.schedule.ranges.skipAfterStart);
        fireEvent.click(getByRole('button', { name: en.schedule.restartOnce }));
        const dialog = within(getByRole('dialog'));
        fireEvent.change(dialog.getByLabelText(en.schedule.onceMinutes), { target: { value: '2000' } });
        fireEvent.click(dialog.getByRole('button', { name: en.schedule.onceSubmit }));
        const rangeError = dialog.getByText(en.schedule.onceRange);
        fireEvent.click(dialog.getByText(en.schedule.onceAt));
        fireEvent.click(dialog.getByRole('button', { name: en.schedule.onceSubmit }));

        expect(skipError).not.toBeNull();
        expect(getByLabelText(en.schedule.skipAfterStart).getAttribute('aria-invalid')).toBe('true');
        expect(rangeError).not.toBeNull();
        expect(dialog.getByText(en.schedule.onceTimeRequired)).not.toBeNull();
        expect(fetch).not.toHaveBeenCalled();
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
