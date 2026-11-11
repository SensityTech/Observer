import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import type { ReactNode } from 'react';
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

// The page links to its settings; no router runs here.
vi.mock('@tanstack/react-router', () => ({
    Link: ({ children, className }: { children: ReactNode; className?: string }) => (
        <a href="/settings/restarts" className={className}>
            {children}
        </a>
    ),
}));

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
    start: null,
    stopHolders: [],
    stop: null,
};

/** The text of what a field is described by, element by element. */
const described = (field: HTMLElement) =>
    (field.getAttribute('aria-describedby') ?? '')
        .split(' ')
        .map((id) => document.getElementById(id)?.textContent)
        .join(' | ');

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

    it('should not call every time moved when the browser lacks the zone and shows UTC', async () => {
        const { getByText, queryByText, live } = await page('1', { ...SCHEDULE, zone: 'Atlantis/Lost_City' });

        expect(getByText(en.schedule.zoneUnknown('Atlantis/Lost_City'))).not.toBeNull();
        expect(queryByText(en.schedule.moved)).toBeNull();
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

    it('should say a single restart waits while FXServer hangs and is being stopped', async () => {
        const { getByRole, getByText, say, live } = await page('9');

        say({ type: 'event', topic: 'server.status', data: { ...ONLINE, state: 'hung' } });
        fireEvent.click(getByRole('button', { name: en.schedule.restartOnce }));

        expect(
            getByText(en.schedule.unavailable(en.schedule.restartOnce, en.schedule.hungOnce)),
        ).not.toBeNull();
        live.stop();
    });

    it('should hand the keyboard to the outcome once the action it pressed is done', async () => {
        const answer = {
            ...SCHEDULE,
            upcoming: SCHEDULE.upcoming
                .slice(0, 1)
                .map((occurrence) => ({ ...occurrence, skipped: { kind: 'manual' as const, by: 'kenji' } })),
        };
        vi.stubGlobal(
            'fetch',
            vi.fn(
                async () =>
                    new Response(JSON.stringify(answer), {
                        status: 200,
                        headers: { 'content-type': 'application/json' },
                    }),
            ),
        );
        const { getByRole, findByText, live } = await page('11', {
            ...SCHEDULE,
            upcoming: SCHEDULE.upcoming.slice(0, 1),
        });

        getByRole('button', { name: en.schedule.skip }).focus();
        fireEvent.click(getByRole('button', { name: en.schedule.skip }));
        // A browser drops the focus of a button it disables; jsdom keeps it.
        act(() => getByRole('button', { name: en.schedule.skip }).blur());
        const outcome = await findByText(en.schedule.skipped('04:00'));

        expect(outcome.closest('[tabindex="-1"]')).toBe(document.activeElement);
        live.stop();
    });

    it('should drop what an action did once the next restart moves on', async () => {
        const planned: ScheduleStatus = {
            ...SCHEDULE,
            next: { at: '2026-10-28T22:15:00Z', source: 'oneOff', by: 'kenji', delayedFrom: null },
            oneOff: { at: '2026-10-28T22:15:00Z', by: 'kenji' },
        };
        vi.stubGlobal(
            'fetch',
            vi.fn(
                async () =>
                    new Response(JSON.stringify(planned), {
                        status: 200,
                        headers: { 'content-type': 'application/json' },
                    }),
            ),
        );
        const { getByRole, findByText, queryByText, publish, live } = await page('19');
        const said = en.schedule.planned('Wed 28 Oct 23:15');

        fireEvent.click(getByRole('button', { name: en.schedule.restartOnce }));
        fireEvent.click(within(getByRole('dialog')).getByRole('button', { name: en.schedule.onceSubmit }));
        const outcome = await findByText(said);
        publish(planned);
        const stays = queryByText(said);
        publish(SCHEDULE);

        expect(outcome).not.toBeNull();
        expect(stays).not.toBeNull();
        expect(queryByText(said)).toBeNull();
        live.stop();
    });

    it('should say what a number or a time needs by its field rather than send what Observer refuses', async () => {
        const fetch = vi.fn();
        vi.stubGlobal('fetch', fetch);
        const { getByRole, live } = await page('19');

        fireEvent.click(getByRole('button', { name: en.schedule.restartOnce }));
        const dialog = within(getByRole('dialog'));
        fireEvent.change(dialog.getByLabelText(en.schedule.onceMinutes), { target: { value: '2000' } });
        fireEvent.click(dialog.getByRole('button', { name: en.schedule.onceSubmit }));
        const rangeError = dialog.getByText(en.schedule.onceRange);
        fireEvent.click(dialog.getByText(en.schedule.onceAt));
        fireEvent.click(dialog.getByRole('button', { name: en.schedule.onceSubmit }));

        expect(rangeError).not.toBeNull();
        expect(dialog.getByText(en.schedule.onceTimeRequired)).not.toBeNull();
        expect(fetch).not.toHaveBeenCalled();
        live.stop();
    });

    it('should tie the hint of a single restart to its field', async () => {
        const { getByRole, live } = await page('19');

        fireEvent.click(getByRole('button', { name: en.schedule.restartOnce }));
        const once = described(within(getByRole('dialog')).getByLabelText(en.schedule.onceMinutes));

        expect(once).toBe(en.schedule.onceHint);
        live.stop();
    });

    it('should lead to its settings, now on the Settings page', async () => {
        const { getByRole, live } = await page('11');

        expect(
            getByRole('link', { name: new RegExp(en.settings.moved(en.settings.categories.restarts.name)) }),
        ).not.toBeNull();
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
