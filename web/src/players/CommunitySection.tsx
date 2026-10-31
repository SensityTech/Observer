import { useQuery } from '@tanstack/react-query';
import { Clock, UserPlus, Users } from 'lucide-react';
import { useId, useState } from 'react';

import type { CommunityTotals } from '../api/generated/CommunityTotals';
import { communityQuery, dailyPoints, weeklyPoints } from '../api/players';
import { ATTRIBUTION } from '../charts/PlayersChart';
import { CommunityChart } from '../charts/CommunityChart';
import { Tile } from '../overview/Tiles';
import { formatCalendarDay } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';

/** A plain difference, signed as the panel signs one (`+12`, `−3h`), with no verdict: these are counts of people. */
const signed = (value: number, words: string) => (value > 0 ? `+${words}` : value < 0 ? `−${words}` : words);

/** The last 7 days against the 7 before, then the chart of the last 30 days. */
export function CommunitySection() {
    const { t, format } = useI18n();
    const words = t.players.community;
    const community = useQuery(communityQuery);
    const tile = (figure: (totals: CommunityTotals) => number, as: (value: number) => string) => {
        if (community.data === undefined) {
            return { value: t.tiles.unknown, facts: [] };
        }
        const now = figure(community.data.week);
        const before = figure(community.data.previousWeek);
        return {
            value: as(now),
            facts: [
                { term: words.before, detail: as(before) },
                { term: words.difference, detail: signed(now - before, as(Math.abs(now - before))) },
            ],
        };
    };
    const period = <p className="self-end text-small text-ink-3">{words.lastWeek}</p>;
    return (
        <>
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-3" aria-busy={community.isPending}>
                <Tile
                    icon={Users}
                    label={words.players}
                    {...tile((totals) => totals.players, format.number)}
                    trend={period}
                />
                <Tile
                    icon={UserPlus}
                    label={words.newPlayers}
                    {...tile((totals) => totals.newPlayers, format.number)}
                    trend={period}
                />
                <Tile
                    icon={Clock}
                    label={words.playTime}
                    {...tile((totals) => totals.playMs, format.playTime)}
                    trend={period}
                />
            </div>
            <CommunityCard />
        </>
    );
}

function CommunityCard() {
    const { t, format, locale } = useI18n();
    const words = t.players.community;
    const titleId = useId();
    const community = useQuery(communityQuery);
    const [weekly, setWeekly] = useState(false);
    const data = community.data;
    const points = data === undefined ? [] : weekly ? weeklyPoints(data) : dailyPoints(data);
    const nobody = data !== undefined && data.days.every((day) => day.players === 0 && day.playMs === 0);

    let summary = '';
    const current = points.at(-1);
    if (data !== undefined && current !== undefined) {
        if (weekly) {
            summary = words.summaryWeeks(
                current.players ?? 0,
                current.newPlayers,
                format.playTime(current.playMs),
            );
        } else {
            const peak = data.days.reduce((best, day) => (day.players > best.players ? day : best));
            summary = words.summaryDays(
                peak.players,
                formatCalendarDay(locale, peak.day, { weekday: false }),
                current.players ?? 0,
                current.newPlayers,
                format.playTime(current.playMs),
            );
        }
    }

    return (
        <section className="card flex flex-col" aria-labelledby={titleId}>
            <div className="card-head">
                <span
                    className="grid size-6 place-items-center rounded-[6px] bg-accent text-on-accent"
                    aria-hidden="true"
                >
                    <Users size={14} strokeWidth={1.75} />
                </span>
                <h2 id={titleId} className="card-title">
                    {words.title}
                </h2>
                <fieldset className="ml-auto inline-flex max-w-full gap-0.5 rounded-control border border-line bg-surface-2 p-0.75 md:-my-1.5">
                    <legend className="sr-only">{words.period}</legend>
                    {[false, true].map((choice) => (
                        <button
                            key={String(choice)}
                            type="button"
                            aria-pressed={weekly === choice}
                            onClick={() => setWeekly(choice)}
                            className="inline-flex h-6.5 items-center rounded-[7px] px-2.5 text-small font-medium text-ink-3 transition-colors hover:text-ink focus-visible:outline-offset-1 aria-pressed:bg-raise aria-pressed:text-ink aria-pressed:shadow-[var(--elev-sm),0_0_0_1px_var(--line)] max-md:h-11 max-md:min-w-11 max-md:justify-center"
                        >
                            {choice ? words.perWeek : words.perDay}
                        </button>
                    ))}
                </fieldset>
            </div>
            <div
                className="flex flex-wrap gap-x-4 gap-y-1.5 border-b border-line px-5 py-2.5 text-small text-ink-2 max-md:px-4"
                aria-hidden="true"
            >
                <span className="flex items-center gap-1.5">
                    <span className="h-0.5 w-3.5 rounded-pill bg-accent" />
                    {words.players}
                </span>
                <span className="flex items-center gap-1.5">
                    <span className="w-3.5 border-t border-ink-2" />
                    {words.newPlayers}
                </span>
                <span className="flex items-center gap-1.5">
                    <span className="h-3 w-[9px] rounded-[2px] bg-chart-muted" />
                    {words.playTime}
                </span>
            </div>
            <div className="card-body flex flex-1 flex-col max-md:p-4">
                {community.isPending ? (
                    <div
                        className="skeleton h-[260px] w-full max-md:h-[200px]"
                        aria-busy="true"
                        aria-label={words.loading}
                    />
                ) : data === undefined ? (
                    <Empty text={words.failed} />
                ) : nobody ? (
                    <Empty text={words.empty} />
                ) : (
                    <figure className="flex flex-col">
                        <CommunityChart points={points} weekly={weekly} />
                        <figcaption className="mt-4 border-t border-line pt-3 text-ui text-ink-3">
                            {summary}
                        </figcaption>
                    </figure>
                )}
            </div>
            <footer className="flex flex-wrap items-center gap-x-1 gap-y-1 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3 max-md:pl-4">
                <span className="mr-auto">{t.players.timeConnected}</span>
                <span>
                    {t.charts.attribution}{' '}
                    <a
                        href={ATTRIBUTION}
                        target="_blank"
                        rel="noreferrer"
                        className="underline-offset-2 hover:text-ink-2 hover:underline"
                    >
                        {t.charts.library}
                    </a>
                </span>
            </footer>
        </section>
    );
}

function Empty({ text }: { text: string }) {
    return (
        <div className="grid h-[260px] place-items-center rounded-card border border-dashed border-line-strong bg-bg-subtle px-6 text-center text-ui text-ink-3 max-md:h-[200px]">
            {text}
        </div>
    );
}
