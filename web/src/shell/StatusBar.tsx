import type { ScheduleStatus } from '../api/generated/ScheduleStatus';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { formatClock, formatDay, zoneDate } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import type { LinkState } from '../live/realtime';
import { cn } from '../lib/cn';
import { secondsSince, useNow } from '../lib/useNow';
import { useSchedule } from '../schedule/context';
import { useServerStatus } from '../server/context';
import { STATE_INK, STATE_LED, isRunning } from '../server/state';

export const LINK_LED = {
    connecting: 'led-pending',
    live: 'led-live',
    offline: 'led-down',
    refused: 'led-down',
} satisfies Record<LinkState, string>;

/** FXServer's state, build, players and next restart, and the live link's, updated in place. */
export function StatusBar() {
    const { t, format } = useI18n();
    const link = useLinkState();
    const { status } = useServerStatus();
    const { schedule, refused } = useSchedule();
    const now = useNow();
    const unknown = <span className="font-normal text-ink-3">{t.status.unknown}</span>;

    const uptime =
        status !== null && isRunning(status.state) && status.startedAt !== null
            ? t.status.upFor(format.duration(secondsSince(status.startedAt, now)))
            : null;
    const players = status?.players ?? null;

    return (
        <dl className="status">
            <div className="status-cell">
                <dt className="md:sr-only">{t.status.state}</dt>
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
                            {uptime !== null && (
                                <span
                                    className={cn(
                                        'font-normal text-ink-3 xl:max-[1261px]:hidden md:max-[941px]:hidden',
                                        // Where the next restart's cell only just fits, it takes the uptime's room.
                                        !refused && 'min-[1320px]:max-[1400px]:hidden',
                                    )}
                                >
                                    · {uptime}
                                </span>
                            )}
                        </>
                    )}
                </dd>
            </div>
            <div className="status-cell">
                <dt>{t.status.build}</dt>
                <dd className="font-mono text-[12.5px]">
                    {status?.build == null ? unknown : String(status.build)}
                </dd>
            </div>
            <div className="status-cell">
                <dt>{t.status.players}</dt>
                <dd className="tabular-nums">
                    {players === null
                        ? unknown
                        : t.status.playerCount(
                              format.number(players.online),
                              players.max === null ? null : format.number(players.max),
                          )}
                </dd>
            </div>
            {!refused && <NextRestartCell schedule={schedule} status={status} now={now} />}
            <div className="status-cell">
                <dt>{t.status.updates}</dt>
                <dd>
                    <span className={cn('led', LINK_LED[link])} aria-hidden="true" />
                    {t.status.link[link]}
                </dd>
            </div>
        </dl>
    );
}

/**
 * When FXServer restarts next, in the schedule's zone: the time alone today there, with its day
 * otherwise, and the time left on the widest screens. Below 1320 px the bar and the server's
 * controls would no longer share a row with it: the Overview and the Schedule page say it there.
 */
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
                <span className="font-normal text-ink-3 max-4xl:hidden">· {t.status.inTime(left)}</span>
            </>
        );
    }
    return (
        <div className="status-cell status-next max-[1320px]:hidden">
            <dt>{t.status.nextRestart}</dt>
            <dd className="tabular-nums">{value}</dd>
        </div>
    );
}
