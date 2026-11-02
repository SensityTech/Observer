import { useQuery } from '@tanstack/react-query';
import { useId } from 'react';

import { seriesQuery } from '../../api/drops';
import type { DropSeries } from '../../api/generated/DropSeries';
import type { EffectRow } from '../../api/generated/EffectRow';
import { effectsQuery } from '../../api/statistics';
import { chipVariants } from '../../components/Chip';
import { VERDICT_ICONS, formatRate, signedPercent, verdictTone } from '../../drops/words';
import { formatDay, knownZone } from '../../i18n/format';
import { useI18n } from '../../i18n/I18nProvider';
import { cn } from '../../lib/cn';
import { useSchedule } from '../../schedule/context';
import { CardState, Scale, shade, weekdays } from './InsightCard';

/** Monday 0 to Sunday 6, the day `instant` falls on in `zone`. */
const weekday = (instant: number, zone: string) => {
    const name = new Intl.DateTimeFormat('en', { weekday: 'short', timeZone: knownZone(zone) }).format(
        instant,
    );
    return ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'].indexOf(name);
};

export type Square = { start: number; rate: number | null; column: number; row: number };

/** The days as squares of a calendar: one column a week from Monday, the first week partial. */
export const calendar = (series: DropSeries, zone: string): Square[] => {
    let column = 0;
    return series.starts.map((start, index) => {
        const row = Math.max(0, weekday(start * 1000, zone));
        if (index > 0 && row === 0) {
            column += 1;
        }
        return { start, rate: series.rates[index] ?? null, column, row };
    });
};

/** Stability over 90 days and what each change did to it, side by side. */
export function StabilityCard() {
    const { t } = useI18n();
    const s = t.statistics.stability;
    const e = t.statistics.effects;
    const calendarTitle = useId();
    const effectsTitle = useId();
    const series = useQuery(seriesQuery('90d', 'unexpected'));
    const effects = useQuery(effectsQuery);
    return (
        <section className="card flex min-w-0 flex-wrap" aria-labelledby={calendarTitle}>
            <div className="flex min-w-0 flex-[1_1_480px] flex-col gap-3 border-line p-5 max-md:p-4 xl:border-r">
                <div className="flex flex-col gap-0.5">
                    <h2 id={calendarTitle} className="card-title">
                        {s.title}
                    </h2>
                    <span className="text-small text-ink-3">{s.meta}</span>
                </div>
                {series.data === undefined || !series.data.rates.some((rate) => rate !== null) ? (
                    <CardState
                        pending={series.isPending}
                        error={series.error}
                        failed={s.failed}
                        empty={s.empty}
                    />
                ) : (
                    <Calendar series={series.data} />
                )}
            </div>
            <section
                className="flex min-w-0 flex-[1_1_400px] flex-col gap-1 p-5 max-md:p-4"
                aria-labelledby={effectsTitle}
            >
                <h2 id={effectsTitle} className="card-title pb-1.5">
                    {e.title}
                </h2>
                {effects.data === undefined || effects.data.rows.length === 0 ? (
                    <CardState
                        pending={effects.isPending}
                        error={effects.error}
                        failed={e.failed}
                        empty={e.empty}
                    />
                ) : (
                    <ul className="flex flex-col">
                        {effects.data.rows.map((row) => (
                            <Effect key={row.at} row={row} />
                        ))}
                    </ul>
                )}
                <p className="pt-2 text-small text-ink-3">{e.note}</p>
                {effects.data !== undefined && !effects.data.changes && (
                    <p className="text-small text-ink-3">{e.buildsOnly}</p>
                )}
            </section>
        </section>
    );
}

function Calendar({ series }: { series: DropSeries }) {
    const { t, locale } = useI18n();
    const s = t.statistics.stability;
    const zone = useSchedule().zone ?? 'UTC';
    const squares = calendar(series, zone);
    const days = weekdays(locale);
    const columns = (squares.at(-1)?.column ?? 0) + 1;
    const rates = squares.map((square) => square.rate).filter((rate): rate is number => rate !== null);
    const max = Math.max(...rates, 0.1);
    const day = (start: number) => formatDay(locale, start * 1000, zone);
    const word = (square: Square) =>
        square.rate === null
            ? s.noPlay(day(square.start))
            : s.day(day(square.start), formatRate(locale, square.rate));
    const columnOf = (at: number) => squares.findLast((square) => square.start <= at)?.column ?? null;
    const worst = squares.reduce<Square | null>(
        (best, square) =>
            square.rate !== null && (best?.rate == null || square.rate > best.rate) ? square : best,
        null,
    );
    const average = rates.reduce((sum, rate) => sum + rate, 0) / Math.max(1, rates.length);
    const summaryId = useId();
    return (
        <figure className="flex flex-col gap-3" aria-labelledby={summaryId}>
            <div className="overflow-x-auto pb-10" aria-hidden="true">
                <div
                    className="relative grid w-max grid-flow-col gap-1"
                    style={{
                        gridTemplateColumns: `34px repeat(${columns}, 20px)`,
                        gridTemplateRows: 'repeat(7, 20px)',
                    }}
                >
                    {days.map((name, row) => (
                        <span
                            key={name}
                            className="font-mono text-micro leading-5 text-ink-3"
                            style={{ gridColumn: 1, gridRow: row + 1 }}
                        >
                            {row % 2 === 0 ? name : ''}
                        </span>
                    ))}
                    {squares.map((square) => (
                        <span
                            key={square.start}
                            title={word(square)}
                            className={cn(
                                'rounded-[4px]',
                                square.rate === null && 'border border-dashed border-line',
                                square === squares.at(-1) && 'ring-2 ring-ink ring-inset',
                            )}
                            style={{
                                gridColumn: square.column + 2,
                                gridRow: square.row + 1,
                                background: square.rate === null ? undefined : shade(square.rate, max),
                            }}
                        />
                    ))}
                    {series.changes.map((change, index) => {
                        const column = columnOf(change.at);
                        // Builds a week apart take turns below the calendar, not to write over each other.
                        const low = index % 2 === 1;
                        return column === null ? null : (
                            <span
                                key={change.at}
                                className={cn(
                                    'pointer-events-none absolute -top-1 border-l-2 border-ink',
                                    low ? '-bottom-9' : '-bottom-5',
                                )}
                                style={{ left: 34 + 4 + column * 24 }}
                            >
                                <span className="absolute -bottom-1 left-1 font-mono text-micro whitespace-nowrap text-ink">
                                    {s.build(change.build === null ? '' : String(change.build))}
                                </span>
                            </span>
                        );
                    })}
                </div>
            </div>
            <figcaption
                id={summaryId}
                className="flex flex-wrap items-center justify-between gap-2 text-small text-ink-3"
            >
                <span>
                    {worst?.rate != null &&
                        s.summary(
                            formatRate(locale, worst.rate),
                            day(worst.start),
                            formatRate(locale, average),
                        )}
                </span>
                <Scale fewer={s.fewer} more={s.more} />
            </figcaption>
            <div className="sr-only">
                <table>
                    <caption>{s.meta}</caption>
                    <tbody>
                        {squares.map((square) => (
                            <tr key={square.start}>
                                <td>{word(square)}</td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            </div>
        </figure>
    );
}

/** A change and what the unexpected drop rate did around it. */
function Effect({ row }: { row: EffectRow }) {
    const { t, locale } = useI18n();
    const e = t.statistics.effects;
    const zone = useSchedule().zone ?? 'UTC';
    const what = row.kinds.includes('resources')
        ? e.resources(row.resources, row.moreResources)
        : row.kinds.includes('build')
          ? e.build(row.build?.to ?? null)
          : row.kinds.includes('cfg')
            ? e.cfg
            : row.kinds.includes('launch')
              ? e.launch
              : e.gameBuild;
    const trend = row.trend;
    const change = trend.change === null ? '' : signedPercent(locale, trend.change);
    const Icon = VERDICT_ICONS[trend.verdict];
    const rate = (value: number | null) =>
        value === null ? t.statistics.instant.unknown : formatRate(locale, value);
    const tone = verdictTone(trend.verdict);
    return (
        <li className="flex flex-wrap items-center gap-x-3 gap-y-1 border-b border-line py-2.5 last:border-b-0">
            <span className="w-14 font-mono text-small text-ink-3">
                {formatDay(locale, row.at, zone, { weekday: false })}
            </span>
            <span className="min-w-0 flex-1 text-ui text-ink [overflow-wrap:anywhere]">{what}</span>
            <span className="font-mono text-small text-ink-2">
                {e.rates(rate(trend.before), rate(trend.now))}
                {row.open && <span className="ml-1 text-ink-3">{e.soFar}</span>}
            </span>
            <span className={chipVariants({ tone: trend.verdict === 'falling' ? 'success' : tone })}>
                <Icon size={12} strokeWidth={2} aria-hidden="true" />
                {e.verdicts[trend.verdict](change)}
            </span>
        </li>
    );
}
