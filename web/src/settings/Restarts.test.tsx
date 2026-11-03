import { act, cleanup, fireEvent, screen, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ScheduleStatus } from '../api/generated/ScheduleStatus';
import { en } from '../i18n/en';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { ScheduleProvider } from '../schedule/context';
import { ServerStatusProvider } from '../server/context';
import { openSettings, settle } from './fixtures';

const items = en.settings.items;
const bar = en.settings.bar;

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
    upcoming: [{ at: '2026-10-29T03:00:00Z', time: '04:00', skipped: null, delayed: null }],
    oneOff: null,
};

/** The text of what a field is described by, element by element. */
const described = (field: HTMLElement) =>
    (field.getAttribute('aria-describedby') ?? '')
        .split(' ')
        .map((id) => document.getElementById(id)?.textContent)
        .join(' | ');

/** The Restarts category for an account holding `permissions`, the schedule as the topic says. */
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
    const view = await openSettings('/settings/restarts', {
        me: { id: 2, username: 'kenji', isOwner: false, permissions, mustChangePassword: false },
        around: (content: ReactNode) => (
            <LiveContext value={live}>
                <ServerStatusProvider>
                    <ScheduleProvider>{content}</ScheduleProvider>
                </ServerStatusProvider>
            </LiveContext>
        ),
    });
    live.start();
    await settle();
    const say = (message: unknown) => act(() => socket?.message(JSON.stringify(message)));
    say({ type: 'hello', connectionId: 1, contract: 1 });
    say({ type: 'snapshot', topic: 'server.schedule', data: schedule });
    const publish = (next: ScheduleStatus) => say({ type: 'event', topic: 'server.schedule', data: next });
    const drop = () => act(() => socket?.close(1006));
    return { ...view, live, publish, say, drop };
};

describe('Restart settings', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should show the settings to an account that sees the schedule, without letting it change them', async () => {
        const { getByText, queryByText, live } = await page('1');
        const enabled = screen.getByRole('switch', { name: items.scheduleEnabled.label }) as HTMLInputElement;

        fireEvent.click(enabled);

        expect(getByText(en.schedule.readOnly)).not.toBeNull();
        expect(enabled.matches(':disabled')).toBe(true);
        expect(queryByText(bar.save)).toBeNull();
        live.stop();
    });

    it('should name each warning in its remove button, one minute in the singular', async () => {
        const { getByRole, live } = await page('11');

        expect(getByRole('button', { name: 'Remove the warning 1 minute before' })).not.toBeNull();
        expect(getByRole('button', { name: 'Remove the warning 30 minutes before' })).not.toBeNull();
        live.stop();
    });

    it('should offer UTC right after the host zone', async () => {
        const { getByLabelText, live } = await page('11');

        const options = [...(getByLabelText(items.timeZone.label) as HTMLSelectElement).options];

        expect(options.slice(0, 2).map((option) => option.value)).toEqual(['', 'UTC']);
        live.stop();
    });

    it('should move the keyboard to the next chip, then to the field, as chips are removed', async () => {
        const { getByRole, getByLabelText, live } = await page('11');

        const sixteen = getByRole('button', { name: en.schedule.removeTime('16:00') });
        getByRole('button', { name: en.schedule.removeTime('04:00') }).focus();
        fireEvent.click(getByRole('button', { name: en.schedule.removeTime('04:00') }));
        const afterFirst = document.activeElement;
        fireEvent.click(sixteen);

        expect(afterFirst).toBe(sixteen);
        expect(document.activeElement).toBe(getByLabelText(en.schedule.addTime));
        live.stop();
    });

    it('should say why a time or a warning was not added, and start the next time afresh', async () => {
        const { getByRole, getByLabelText, getByText, getAllByRole, live } = await page('11');
        const add = (index: number) => {
            const button = getAllByRole('button', { name: en.schedule.add })[index];
            if (button !== undefined) {
                fireEvent.click(button);
            }
        };

        fireEvent.change(getByLabelText(en.schedule.addTime), { target: { value: '' } });
        add(0);
        const incomplete = getByText(en.schedule.timeIncomplete);
        fireEvent.change(getByLabelText(en.schedule.addTime), { target: { value: '07:30' } });
        fireEvent.keyDown(getByLabelText(en.schedule.addTime), { key: 'Enter' });
        const added = within(getByRole('list', { name: en.schedule.times })).queryByText('07:30');
        const fresh = getByLabelText(en.schedule.addTime);
        fireEvent.change(getByLabelText(en.schedule.addWarning), { target: { value: '0' } });
        add(1);
        const outOfRange = getByText(en.schedule.ranges.warning);
        fireEvent.change(getByLabelText(en.schedule.addWarning), { target: { value: '30' } });
        add(1);

        expect(incomplete).not.toBeNull();
        expect(added).not.toBeNull();
        expect((fresh as HTMLInputElement).value).toBe('');
        expect(document.activeElement).toBe(fresh);
        expect(outOfRange).not.toBeNull();
        expect(getByText(en.schedule.listedAlready)).not.toBeNull();
        live.stop();
    });

    it('should count the changes typed in the save bar, mark them, and say they apply at once', async () => {
        const { getByRole, getByText, queryByText, live } = await page('11');
        expect(queryByText(bar.changes(1))).toBeNull();

        fireEvent.click(getByRole('button', { name: en.schedule.removeTime('16:00') }));
        fireEvent.click(getByRole('switch', { name: items.scheduleEnabled.label }));

        expect(getByText(bar.changes(2))).not.toBeNull();
        expect(getByText(`· ${bar.applies([en.settings.appliesWhen.now])}`)).not.toBeNull();
        expect(getByText(items.scheduleTimes.label).parentElement?.textContent).toContain(
            en.settings.changed,
        );
        fireEvent.click(getByRole('button', { name: bar.discard }));
        expect(queryByText(bar.changes(2))).toBeNull();
        expect(queryByText(en.settings.changed)).toBeNull();
        live.stop();
    });

    it('should keep the settings typed while the live link drops and comes back', async () => {
        const { getByRole, getByText, queryByText, say, drop, live } = await page('11');
        fireEvent.click(getByRole('button', { name: en.schedule.removeTime('16:00') }));

        drop();
        const down = queryByText(en.schedule.outOfDate);
        const keptDown = queryByText(bar.changes(1));
        // The client tries again after a second.
        await act(() => new Promise((resolve) => setTimeout(resolve, 1_200)));
        say({ type: 'hello', connectionId: 2, contract: 1 });
        say({ type: 'snapshot', topic: 'server.schedule', data: SCHEDULE });

        expect(down).not.toBeNull();
        expect(keptDown).not.toBeNull();
        expect(getByText(bar.changes(1))).not.toBeNull();
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

        fireEvent.click(getByRole('switch', { name: items.scheduleEnabled.label }));
        fireEvent.click(getByRole('button', { name: bar.save }));

        expect(await findByText(`04:00: ${en.schedule.problems.tooClose}`)).not.toBeNull();
        expect(fetch).toHaveBeenCalledOnce();
        live.stop();
    });

    it('should say what a number needs by its field rather than send what Observer refuses', async () => {
        const fetch = vi.fn();
        vi.stubGlobal('fetch', fetch);
        Element.prototype.scrollIntoView = () => {};
        const { getByRole, getByLabelText, getByText, live } = await page('19');

        fireEvent.change(getByLabelText(items.skipAfterStart.label), { target: { value: '1.5' } });
        fireEvent.click(getByRole('button', { name: bar.save }));

        expect(getByText(en.schedule.ranges.skipAfterStart)).not.toBeNull();
        expect(getByLabelText(items.skipAfterStart.label).getAttribute('aria-invalid')).toBe('true');
        expect(fetch).not.toHaveBeenCalled();
        live.stop();
    });

    it('should tie each explanation, hint and error to the fields it is about', async () => {
        Element.prototype.scrollIntoView = () => {};
        const { getByRole, getByLabelText, live } = await page('19');

        const zone = described(getByLabelText(items.timeZone.label));
        fireEvent.click(getByRole('radio', { name: en.schedule.modes.interval }));
        fireEvent.change(getByLabelText(en.schedule.intervalMinutes), { target: { value: '1.5' } });
        fireEvent.click(getByRole('button', { name: bar.save }));
        const fields = [
            getByLabelText(en.schedule.intervalHours),
            getByLabelText(en.schedule.intervalMinutes),
        ];

        expect(zone).toBe(`${items.timeZone.about} | ${en.schedule.timeZoneHint}`);
        for (const field of fields) {
            expect(field.getAttribute('aria-invalid')).toBe('true');
            expect(described(field)).toBe(
                `${en.schedule.ranges.intervalMinutes} | ${en.schedule.intervalHint}`,
            );
        }
        live.stop();
    });
});
