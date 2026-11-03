import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { TriangleAlert } from 'lucide-react';
import { useId } from 'react';

import type { DropsRange } from '../../api/generated/DropsRange';
import type { RunGrowth } from '../../api/generated/RunGrowth';
import type { RunMemory } from '../../api/generated/RunMemory';
import type { Timeline } from '../../api/generated/Timeline';
import { growthQuery } from '../../api/metrics';
import { runsQuery, timelineQuery } from '../../api/statistics';
import { Chip } from '../../components/Chip';
import { formatTimestamp } from '../../i18n/format';
import { useI18n } from '../../i18n/I18nProvider';
import { forecast, rowsOf } from '../lanes';
import { CardState, InsightCard } from './InsightCard';

const WIDTH = 600;
const HEIGHT = 230;
const LEFT = 10;
const RIGHT = 590;
const TOP = 14;
const BOTTOM = 196;
const POINTS = 300;
/** The open run's forecast reaches this far, without a restart. */
const AHEAD = 24 * 3600;

type Path = string;

/** A line through the measured points of `rows`, broken where one is missing. */
const linePath = (
    rows: { time: number; value: number | null }[],
    x: (time: number) => number,
    y: (value: number) => number,
): Path => {
    let path = '';
    let open = false;
    for (const row of rows) {
        if (row.value === null) {
            open = false;
            continue;
        }
        path += `${open ? 'L' : 'M'}${x(row.time).toFixed(1)} ${y(row.value).toFixed(1)} `;
        open = true;
    }
    return path.trim();
};

export function MemoryCard({ range }: { range: DropsRange }) {
    const { t, format } = useI18n();
    const m = t.statistics.memory;
    const runs = useQuery({ ...runsQuery(range), placeholderData: keepPreviousData });
    const window = runs.data;
    const curve = useQuery({
        ...timelineQuery(window?.from ?? 0, window?.to ?? 0, ['fxsPrivate'], POINTS),
        enabled: window !== undefined,
        placeholderData: keepPreviousData,
    });
    const growth = useQuery(growthQuery);
    const open = growth.data ?? null;
    const leak = open?.private?.leak === true;
    const acceleration = window?.acceleration ?? null;
    const foot =
        window === undefined
            ? undefined
            : acceleration === null
              ? m.fewRuns
              : acceleration.accelerating
                ? m.accelerating(format.bytes(acceleration.perRun, { signed: true }))
                : m.steady;
    const measured = curve.data?.series[0]?.values.some((value) => value !== null) ?? false;
    return (
        <InsightCard
            title={m.title}
            meta={m.meta(t.drops.ranges[range])}
            chip={
                leak ? (
                    <Chip tone="warning" icon={TriangleAlert}>
                        {m.leak}
                    </Chip>
                ) : undefined
            }
            foot={foot}
            stale={runs.isPlaceholderData || curve.isPlaceholderData}
        >
            {window === undefined || curve.data === undefined || !measured ? (
                <CardState
                    pending={runs.isPending || (window !== undefined && curve.isPending)}
                    error={runs.error ?? curve.error}
                    failed={m.failed}
                    empty={window !== undefined && window.runs.length === 0 ? m.empty : m.empty}
                />
            ) : (
                <Runs memory={window} curve={curve.data} growth={open} />
            )}
        </InsightCard>
    );
}

function Runs({ memory, curve, growth }: { memory: RunMemory; curve: Timeline; growth: RunGrowth | null }) {
    const { t, format } = useI18n();
    const m = t.statistics.memory;
    const summaryId = useId();
    const rows = rowsOf(curve, 'fxsPrivate');
    const fitted = growth?.private ?? null;
    const projected =
        growth !== null && fitted !== null
            ? forecast(
                  fitted,
                  growth.from,
                  memory.to,
                  memory.to + AHEAD,
                  Math.max(curve.step, 900),
                  memory.to,
              )
            : null;
    const end = projected === null ? memory.to : memory.to + AHEAD;
    const values = [
        ...rows.map((row) => row.value),
        ...(projected?.cone.flatMap((point) => [point.min, point.max]) ?? []),
    ].filter((value): value is number => value !== null);
    const low = Math.min(...values);
    const high = Math.max(...values);
    const margin = (high - low) * 0.1 || high * 0.05 || 1;
    const [floor, ceiling] = [Math.max(0, low - margin), high + margin];
    const x = (time: number) => LEFT + ((time - memory.from) / (end - memory.from)) * (RIGHT - LEFT);
    const y = (value: number) => BOTTOM - ((value - floor) / (ceiling - floor)) * (BOTTOM - TOP);
    const open = memory.runs.findLast((run) => run.endedAt === null) ?? null;
    const openStart = open?.startedAt ?? null;
    const before = rows.filter((row) => openStart === null || row.time < openStart);
    const current = rows.filter((row) => openStart !== null && row.time >= openStart);
    const cone =
        projected === null
            ? null
            : `M${projected.cone
                  .filter((point) => point.max !== null)
                  .map((point) => `${x(point.time).toFixed(1)} ${y(point.max ?? 0).toFixed(1)}`)
                  .join(' L')} L${projected.cone
                  .filter((point) => point.min !== null)
                  .toReversed()
                  .map((point) => `${x(point.time).toFixed(1)} ${y(point.min ?? 0).toFixed(1)}`)
                  .join(' L')} Z`;
    const ahead = projected?.trend.at(-1)?.value ?? null;
    const grid = [floor + (ceiling - floor) * 0.25, floor + (ceiling - floor) * 0.75];
    const words = [
        m.summary(memory.runs.length),
        ...memory.runs.flatMap((run) =>
            run.private === null
                ? []
                : [
                      m.runRate(
                          formatTimestamp(new Date(run.startedAt * 1000), { seconds: false }),
                          format.bytes(run.private.perHour, { signed: true }),
                      ),
                  ],
        ),
        ...(ahead === null ? [] : [m.forecast(format.bytes(ahead), format.duration(AHEAD))]),
    ];
    return (
        <figure className="flex flex-col gap-2">
            <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} className="w-full" aria-hidden="true">
                {grid.map((value) => (
                    <g key={value}>
                        <line x1={LEFT} x2={RIGHT} y1={y(value)} y2={y(value)} className="stroke-line" />
                        <text
                            x={RIGHT}
                            y={y(value) - 4}
                            textAnchor="end"
                            className="fill-ink-3 font-mono text-micro"
                        >
                            {format.bytes(value)}
                        </text>
                    </g>
                ))}
                {projected !== null && (
                    <rect
                        x={x(memory.to)}
                        y={TOP - 4}
                        width={RIGHT - x(memory.to)}
                        height={BOTTOM - TOP + 4}
                        className="fill-bg-subtle"
                    />
                )}
                {memory.runs.map((run) =>
                    run.startedAt > memory.from ? (
                        <line
                            key={run.startedAt}
                            x1={x(run.startedAt)}
                            x2={x(run.startedAt)}
                            y1={TOP - 4}
                            y2={BOTTOM}
                            className="stroke-ink-3"
                            strokeDasharray="2 3"
                        />
                    ) : null,
                )}
                {cone !== null && <path d={cone} className="fill-accent/15" />}
                <path d={linePath(before, x, y)} fill="none" className="stroke-ink-2" strokeWidth={1.5} />
                <path d={linePath(current, x, y)} fill="none" className="stroke-accent" strokeWidth={2} />
                {projected !== null && (
                    <path
                        d={linePath(projected.trend, x, y)}
                        fill="none"
                        className="stroke-accent"
                        strokeWidth={1.5}
                        strokeDasharray="4 4"
                    />
                )}
                {ahead !== null && (
                    <text
                        x={RIGHT - 4}
                        y={y(ahead) - 8}
                        textAnchor="end"
                        className="fill-ink font-mono text-eyebrow font-semibold"
                    >
                        {format.bytes(ahead)}
                    </text>
                )}
                {memory.runs.map((run) => {
                    // A run cut by the range's start has too little of it drawn to carry its rate.
                    if (run.private === null || run.startedAt < memory.from) {
                        return null;
                    }
                    const middle = (Math.max(run.startedAt, memory.from) + (run.endedAt ?? memory.to)) / 2;
                    const near = rows.reduce(
                        (best, row) =>
                            row.value !== null && Math.abs(row.time - middle) < Math.abs(best.time - middle)
                                ? row
                                : best,
                        rows[0] ?? { time: middle, value: null },
                    );
                    if (near.value === null) {
                        return null;
                    }
                    return (
                        <text
                            key={run.startedAt}
                            x={x(middle)}
                            y={y(near.value) - 10}
                            textAnchor="middle"
                            className={
                                run.endedAt === null
                                    ? 'fill-accent-ink font-mono text-micro'
                                    : 'fill-ink-3 font-mono text-micro'
                            }
                        >
                            {m.rate(format.bytes(run.private.perHour, { signed: true }))}
                        </text>
                    );
                })}
                <line x1={LEFT} x2={RIGHT} y1={BOTTOM} y2={BOTTOM} className="stroke-line-strong" />
                <text x={LEFT} y={HEIGHT - 12} className="fill-ink-3 font-mono text-micro">
                    {formatTimestamp(new Date(memory.from * 1000), { seconds: false })}
                </text>
                <text
                    x={x(memory.to)}
                    y={HEIGHT - 12}
                    textAnchor="middle"
                    className="fill-ink font-mono text-micro"
                >
                    {t.statistics.timeline.now}
                </text>
            </svg>
            <figcaption id={summaryId} className="text-small text-ink-3">
                {words.join(' ')}
                {memory.runs.some((run) => run.startedAt < memory.fittedFrom) && (
                    <>
                        {' '}
                        {m.fittedFrom(
                            formatTimestamp(new Date(memory.fittedFrom * 1000), { seconds: false }),
                        )}
                    </>
                )}
            </figcaption>
        </figure>
    );
}
