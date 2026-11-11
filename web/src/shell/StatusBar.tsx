import type { ScheduleStatus } from '../api/generated/ScheduleStatus';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { formatClock, formatDay, zoneDate } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import type { LinkState } from '../live/realtime';
import { cn } from '../lib/cn';
import { secondsSince, useNow } from '../lib/useNow';
import { useArtifacts } from '../artifacts/context';
import { useSchedule } from '../schedule/context';
import { startTotal } from '../overview/StartCard';
import { useServerStatus } from '../server/context';
import { STATE_INK, STATE_LED, isRunning, waitedFor } from '../server/state';

export const LINK_LED = {
    connecting: 'led-pending',
    live: 'led-live',
    offline: 'led-down',
    refused: 'led-down',
} satisfies Record<LinkState, string>;

/**
 * FXServer's state and for how long, its build and its next restart, updated in place. The live
 * link shows only while it is not live: the figures above every page then stop moving.
 */
export function StatusBar() {
    const { t, format } = useI18n();
    const link = useLinkState();
    const { status } = useServerStatus();
    const { schedule, refused } = useSchedule();
    const { status: builds } = useArtifacts();
    const now = useNow();
    const complement =
        builds === null
            ? null
            : builds.running !== null &&
                builds.probation !== null &&
                builds.probation.build === builds.running
              ? t.status.probation
              : builds.selected !== null && builds.selected !== builds.running
                ? t.status.nextBuild(String(builds.selected))
                : null;
    const unknown = <span className="font-normal text-ink-3">{t.status.unknown}</span>;

    let lasted = null;
    if (status !== null && status.state === 'starting') {
        const started = status.start;
        lasted = [
            t.status.forTime(format.elapsed(Math.max(0, now - Date.parse(status.since)) / 1000)),
            ...(started === null ? [] : [t.status.resources(started.started, startTotal(started))]),
        ].join(' · ');
    } else if (status !== null && isRunning(status.state) && status.startedAt !== null) {
        lasted = t.status.upFor(format.duration(secondsSince(status.startedAt, now)));
    } else if (status !== null && !isRunning(status.state)) {
        const waiting = waitedFor(t, status);
        lasted = [
            t.status.forTime(format.duration(secondsSince(status.since, now))),
            ...(waiting === null ? [] : [waiting]),
        ].join(' · ');
    }

    return (
        <dl className="status">
            <div className="status-cell">
                <dt className="sr-only">{t.status.state}</dt>
                <dd>
                    {status === null ? (
                        <>
                            <span className="led led-off" aria-hidden="true" />
                            {unknown}
                        </>
                    ) : (
                        <>
                            <span className={cn('led', STATE_LED[status.state])} aria-hidden="true" />
                            <span className={cn('font-semibold', STATE_INK[status.state])}>
                                {t.server_state.states[status.state]}
                            </span>
                            {lasted !== null && <span className="font-normal text-ink-3"> · {lasted}</span>}
                        </>
                    )}
                </dd>
            </div>
            <div className="status-cell">
                <dt>{t.status.build}</dt>
                <dd>
                    {status === null ? (
                        unknown
                    ) : status.build === null ? (
                        <span className="font-normal text-ink-3">{t.status.none}</span>
                    ) : (
                        <span className="font-mono text-[12.5px]">{String(status.build)}</span>
                    )}
                    {complement !== null && <span className="font-normal text-ink-3"> · {complement}</span>}
                </dd>
            </div>
            {!refused && <NextRestartCell schedule={schedule} status={status} now={now} />}
            {link !== 'live' && (
                <div className="status-cell">
                    <dt>{t.status.linkLabel}</dt>
                    <dd>
                        <span className={cn('led', LINK_LED[link])} aria-hidden="true" />
                        {t.status.link[link]}
                    </dd>
                </div>
            )}
        </dl>
    );
}

/** When FXServer restarts next, in the schedule's zone: the time alone today there, with its day otherwise. */
function NextRestartCell({
    schedule,
    status,
    now,
}: {
    schedule: ScheduleStatus | null;
    status: ServerStatus | null;
    now: number;
}) {
    const { t, locale, format } = useI18n();
    const next = schedule?.next ?? null;
    const running = status !== null && isRunning(status.state);
    let value;
    if (schedule === null) {
        value = <span className="font-normal text-ink-3">{t.status.unknown}</span>;
    } else if (next === null || !running) {
        value = <span className="font-normal text-ink-3">{t.status.none}</span>;
    } else {
        const clock = formatClock(locale, next.at, schedule.zone);
        const today = zoneDate(next.at, schedule.zone) === zoneDate(now, schedule.zone);
        const left = format.duration(Math.max(0, Date.parse(next.at) - now) / 1000);
        value = (
            <>
                <span className="font-mono text-[12.5px]">
                    {today
                        ? clock
                        : t.schedule.dayTime(
                              formatDay(locale, next.at, schedule.zone, { weekday: false }),
                              clock,
                          )}
                </span>
                <span className="font-normal text-ink-3"> · {t.status.inTime(left)}</span>
            </>
        );
    }
    return (
        <div className="status-cell status-next">
            <dt>{t.status.nextRestart}</dt>
            <dd className="tabular-nums">{value}</dd>
        </div>
    );
}
