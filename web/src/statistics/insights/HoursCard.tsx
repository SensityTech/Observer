import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';

import type { PlayerHours } from '../../api/generated/PlayerHours';
import { Permission } from '../../api/generated/Permission';
import { hoursQuery } from '../../api/statistics';
import { allows, useMe } from '../../auth/me';
import { buttonVariants } from '../../components/Button';
import { useI18n } from '../../i18n/I18nProvider';
import { useSchedule } from '../../schedule/context';
import { hourLabel, restartAdvice } from '../advice';
import { CardState, InsightCard, Scale, shade, weekdays } from './InsightCard';

const HOURS = Array.from({ length: 24 }, (_, hour) => hour);

export function HoursCard() {
    const { t } = useI18n();
    const h = t.statistics.hours;
    const query = useQuery(hoursQuery);
    const data = query.data;
    const measured = data?.players.some((value) => value !== null) ?? false;
    return (
        <InsightCard
            title={h.title}
            meta={h.meta}
            foot={data !== undefined && measured ? <Advice hours={data} /> : undefined}
        >
            {data === undefined || !measured ? (
                <CardState pending={query.isPending} error={query.error} failed={h.failed} empty={h.empty} />
            ) : (
                <Heat hours={data} />
            )}
        </InsightCard>
    );
}

function Heat({ hours }: { hours: PlayerHours }) {
    const { t, locale, format } = useI18n();
    const h = t.statistics.hours;
    const { schedule } = useSchedule();
    const days = weekdays(locale);
    const max = Math.max(...hours.players.map((value) => value ?? 0), 1);
    const planned = new Set(
        schedule?.settings.enabled && schedule.settings.mode === 'fixedTimes'
            ? schedule.settings.times.map((time) => Number(time.slice(0, 2)))
            : [],
    );
    const word = (day: number, hour: number) => {
        const players = hours.players[day * 24 + hour] ?? null;
        return players === null
            ? h.unmeasured(days[day] ?? '', hourLabel(hour))
            : h.cell(days[day] ?? '', hourLabel(hour), format.number(Math.round(players)));
    };
    const quietDay =
        hours.quietest === null
            ? null
            : [0, 1, 2, 3, 4, 5, 6].reduce((best, day) =>
                  (hours.players[day * 24 + (hours.quietest ?? 0)] ?? Infinity) <
                  (hours.players[best * 24 + (hours.quietest ?? 0)] ?? Infinity)
                      ? day
                      : best,
              );
    return (
        <figure className="flex flex-col gap-2">
            <div className="flex flex-col gap-[3px] overflow-x-auto overflow-y-hidden" aria-hidden="true">
                <div className="grid h-6 grid-cols-[34px_repeat(24,minmax(9px,1fr))] gap-[3px]">
                    <span />
                    {HOURS.map((hour) => (
                        <span key={hour} className="relative">
                            {(hour === hours.quietest || planned.has(hour)) && (
                                <span
                                    className={
                                        hour === hours.quietest
                                            ? 'absolute bottom-0.5 left-1/2 -translate-x-1/2 rounded-pill bg-success-soft px-1.5 font-mono text-tag whitespace-nowrap text-success'
                                            : 'absolute bottom-0.5 left-1/2 -translate-x-1/2 rounded-pill bg-surface-3 px-1.5 font-mono text-tag whitespace-nowrap text-ink'
                                    }
                                >
                                    {hour === hours.quietest ? h.quietest : h.restartNow}
                                </span>
                            )}
                        </span>
                    ))}
                </div>
                {days.map((day, row) => (
                    <div
                        key={day}
                        className="grid h-6 grid-cols-[34px_repeat(24,minmax(9px,1fr))] gap-[3px] max-md:h-5"
                    >
                        <span className="font-mono text-meta leading-6 text-ink-3 max-md:leading-5">
                            {day}
                        </span>
                        {HOURS.map((hour) => {
                            const players = hours.players[row * 24 + hour] ?? null;
                            const ringed = hour === hours.quietest && row === quietDay;
                            return (
                                <span
                                    key={hour}
                                    title={word(row, hour)}
                                    className={
                                        players === null
                                            ? 'rounded-[4px] border border-dashed border-line'
                                            : ringed
                                              ? 'rounded-[4px] ring-2 ring-ink ring-inset'
                                              : 'rounded-[4px]'
                                    }
                                    style={players === null ? undefined : { background: shade(players, max) }}
                                />
                            );
                        })}
                    </div>
                ))}
                <div className="grid grid-cols-[34px_repeat(24,minmax(9px,1fr))] gap-[3px]">
                    <span />
                    {HOURS.map((hour) => (
                        <span key={hour} className="font-mono text-tag text-ink-3">
                            {hour % 6 === 0 ? String(hour).padStart(2, '0') : ''}
                        </span>
                    ))}
                </div>
            </div>
            <figcaption className="flex flex-wrap items-center justify-between gap-2 text-small text-ink-3">
                <span>{h.zone(hours.zone)}</span>
                <Scale fewer={h.fewer} more={h.more} />
            </figcaption>
            <div className="sr-only">
                <table>
                    <caption>{h.meta}</caption>
                    <thead>
                        <tr>
                            <th scope="col">{h.zone(hours.zone)}</th>
                            {HOURS.map((hour) => (
                                <th key={hour} scope="col">
                                    {hourLabel(hour)}
                                </th>
                            ))}
                        </tr>
                    </thead>
                    <tbody>
                        {days.map((day, row) => (
                            <tr key={day}>
                                <th scope="row">{day}</th>
                                {HOURS.map((hour) => {
                                    const players = hours.players[row * 24 + hour] ?? null;
                                    return (
                                        <td key={hour}>
                                            {players === null
                                                ? t.statistics.instant.unknown
                                                : format.number(Math.round(players))}
                                        </td>
                                    );
                                })}
                            </tr>
                        ))}
                    </tbody>
                </table>
            </div>
        </figure>
    );
}

/** What the map says about the schedule, and the way to the schedule when a move is worth it. */
function Advice({ hours }: { hours: PlayerHours }) {
    const { t, format } = useI18n();
    const h = t.statistics.hours;
    const me = useMe();
    const { schedule } = useSchedule();
    const advice = restartAdvice(hours, schedule?.settings ?? null);
    const players = (value: number) => format.number(Math.round(value));
    const planning = allows(me, Permission.ScheduleManage);
    switch (advice.kind) {
        case 'none':
            return null;
        case 'quiet':
            return <>{h.quiet(hourLabel(advice.quiet), players(advice.players))}</>;
        case 'interval':
            return <>{h.interval(hourLabel(advice.quiet), players(advice.players))}</>;
        case 'already':
            return <>{h.already(advice.time)}</>;
        case 'close':
            return <>{h.close(advice.time, players(advice.players))}</>;
        case 'move':
            return (
                <span className="flex flex-wrap items-center gap-3">
                    <span className="min-w-0 flex-1">
                        {h.advice(
                            advice.time,
                            players(advice.players),
                            hourLabel(advice.quiet),
                            players(advice.after),
                        )}
                    </span>
                    {planning && (
                        <Link to="/schedule" className={buttonVariants({ size: 'sm' })}>
                            {h.move(hourLabel(advice.quiet))}
                        </Link>
                    )}
                </span>
            );
    }
}
