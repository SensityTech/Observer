import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { Plus, RotateCcw, Waves } from 'lucide-react';
import { Suspense, lazy, useId, useMemo, useState } from 'react';

import type { Range } from '../api/generated/Range';
import type { Timeline } from '../api/generated/Timeline';
import { growthQuery } from '../api/metrics';
import { eventsQuery, isFeatureDisabled, timelineQuery } from '../api/statistics';
import { downPeriods, downTime } from '../charts/coverage';
import { Button } from '../components/Button';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { errorMessage } from '../i18n/errors';
import { formatTimestamp } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { CodeLoadError } from '../lib/lazyCode';
import { useNow } from '../lib/useNow';
import { useLiveMetrics } from '../metrics/live';
import { eventsAround, linksIn } from './events';
import { EventLine, InstantPanel, type EventWordings } from './InstantPanel';
import {
    BASE_LANES,
    FORECAST_SHARE,
    bucketAt,
    forecast,
    frameLine,
    hitches,
    newestBucket,
    seriesOf,
    timeOf,
    valuesOf,
    type LaneKey,
} from './lanes';
import {
    DEFAULT_TIMELINE_RANGE,
    EXTRA_LANES,
    TIMELINE_RANGES,
    lanesParam,
    timelineWindow,
    type ExtraLane,
    type StatisticsSearch,
} from './search';
import type { TimelineChartProps } from './TimelineChart';

// The chart and lightweight-charts load with the tab, as the Overview's charts do.
const TimelineChart = lazy(() =>
    import('./TimelineChart').catch((cause: unknown) => {
        throw new CodeLoadError(cause);
    }),
);

const WEEK = 7 * 86_400;
/** A window that ends now moves on this often, which reads it again. */
const LIVE_STEP = 30;
const TABLE_POINTS = 48;
const TABLE_SERIES = ['players', 'mainTick', 'fxsCpu', 'fxsPrivate'] as const;
const LISTED = 12;
const LONGEST = 400 * 86_400;

/** The timeline's tab: every lane on one time axis, the panel at the cursor, and what happened. */
export function TimelineTab({
    search,
    go,
}: {
    search: StatisticsSearch;
    go: (next: StatisticsSearch, replace?: boolean) => void;
}) {
    const { t, format } = useI18n();
    const s = t.statistics.timeline;
    const nowMs = useNow(LIVE_STEP * 1000);
    const now = Math.floor(nowMs / 1000 / LIVE_STEP) * LIVE_STEP;
    const window = timelineWindow(search, now);
    const added = search.lanes;
    const extra = useMemo(() => added ?? [], [added]);
    const lanes = useMemo<LaneKey[]>(() => [...BASE_LANES, ...extra], [extra]);
    const series = useMemo(() => seriesOf(lanes), [lanes]);
    const comparing = search.compare === true;

    const main = useQuery({
        ...timelineQuery(window.from, window.to, series),
        placeholderData: keepPreviousData,
    });
    const compare = useQuery({
        ...timelineQuery(window.from - WEEK, window.to - WEEK, series),
        enabled: comparing,
        placeholderData: keepPreviousData,
    });
    const events = useQuery({ ...eventsQuery(window.from, window.to), placeholderData: keepPreviousData });
    const growth = useQuery(growthQuery);
    const { latest } = useLiveMetrics();
    const [cursor, setCursor] = useState<number | null>(null);
    const [table, setTable] = useState(false);
    const titleId = useId();
    const summaryId = useId();

    const timeline = main.data ?? null;
    const list = events.data?.events ?? [];
    const periods = useMemo(() => (timeline === null ? [] : downPeriods(timeline)), [timeline]);
    const projected = useMemo(() => {
        const fitted = growth.data?.private ?? null;
        if (timeline === null || window.zoomed || fitted === null || growth.data == null) {
            return null;
        }
        const until = now + Math.round((window.to - window.from) * FORECAST_SHARE);
        return forecast(
            fitted,
            growth.data.from,
            now,
            until,
            timeline.step,
            Math.max(growth.data.from, timeline.from),
        );
    }, [timeline, growth.data, now, window.from, window.to, window.zoomed]);

    // Which feature, and what holds it off: observer.toml, or one it needs that does not run.
    if (isFeatureDisabled(main.error)) {
        return <Notice>{errorMessage(t, main.error)}</Notice>;
    }

    const words: EventWordings = {
        duration: (seconds) => format.duration(seconds),
        clock: (seconds) => format.dateTime(new Date(seconds * 1000), { dateStyle: undefined }),
        ms: (ms) => format.milliseconds(ms),
    };
    const range = search.range ?? DEFAULT_TIMELINE_RANGE;
    const zoom = (from: number, to: number) => {
        const span = Math.min(LONGEST, Math.max(60, to - from));
        const start = Math.max(1, Math.min(from, now - span));
        go({ ...search, from: start, to: Math.min(start + span, now) });
        setCursor(null);
    };
    const reset = () => {
        go({ ...search, from: undefined, to: undefined });
        setCursor(null);
    };
    const toggleLane = (lane: ExtraLane) =>
        go(
            {
                ...search,
                lanes: lanesParam(
                    extra.includes(lane) ? extra.filter((each) => each !== lane) : [...extra, lane],
                ),
            },
            true,
        );

    const bucket = timeline === null ? 0 : (cursor ?? newestBucket(timeline, 'players'));
    const cursorTime = timeline === null ? now : timeOf(timeline, bucket);
    const span = window.to - window.from;
    const around = eventsAround(list, cursorTime, span);
    const links = linksIn(events.data?.links ?? [], list);
    const measured = timeline !== null && timeline.observed.some((count) => count > 0);
    const stale = main.isPlaceholderData;

    return (
        <div className="flex flex-col gap-3">
            <div className="flex flex-wrap items-center gap-3">
                <Segmented
                    label={s.range}
                    choices={TIMELINE_RANGES}
                    value={range}
                    words={t.charts.ranges}
                    onChange={(choice) =>
                        go({
                            ...search,
                            range: choice === DEFAULT_TIMELINE_RANGE ? undefined : choice,
                            from: undefined,
                            to: undefined,
                        })
                    }
                />
                <Button
                    size="sm"
                    icon={Waves}
                    aria-pressed={comparing}
                    className="aria-pressed:border-ink aria-pressed:bg-surface-2"
                    onClick={() => go({ ...search, compare: comparing ? undefined : true }, true)}
                >
                    {s.compare}
                </Button>
                {window.zoomed && (
                    <>
                        <span className="font-mono text-small text-ink-2">
                            {s.zoomed(
                                formatTimestamp(new Date(window.from * 1000), { seconds: false }),
                                formatTimestamp(new Date(window.to * 1000), { seconds: false }),
                            )}
                        </span>
                        <Button size="sm" tone="ghost" icon={RotateCcw} onClick={reset}>
                            {s.resetZoom}
                        </Button>
                    </>
                )}
                {!window.zoomed && <span className="text-small text-ink-3 md:ml-auto">{s.every}</span>}
            </div>
            <div className="flex flex-wrap items-stretch gap-3">
                <section className="card flex min-w-0 flex-[999_1_640px] flex-col" aria-labelledby={titleId}>
                    <div className="card-head">
                        <h2 id={titleId} className="card-title">
                            {s.title}
                        </h2>
                        <span className="text-meta text-ink-3">{s.meta}</span>
                    </div>
                    <div className="card-body flex flex-col gap-3 max-md:p-4">
                        {main.isPending ? (
                            <div
                                className="skeleton h-[480px] w-full"
                                aria-busy="true"
                                aria-label={s.loading}
                            />
                        ) : timeline === null ? (
                            <Empty text={s.failed} />
                        ) : !measured ? (
                            <Empty text={s.empty} />
                        ) : (
                            <div
                                className={cn('transition-opacity', stale && 'opacity-50')}
                                aria-busy={stale}
                            >
                                <ErrorBoundary fallback={<Empty text={s.failed} />}>
                                    <Suspense fallback={<div className="skeleton h-[480px] w-full" />}>
                                        <TimelineChart
                                            {...{
                                                timeline,
                                                compare: comparing ? (compare.data ?? null) : null,
                                                lanes,
                                                events: list,
                                                forecast: projected,
                                                periods,
                                                now: window.zoomed ? null : now,
                                                cursor,
                                                onCursor: setCursor,
                                                onZoom: zoom,
                                                onReset: reset,
                                                onEvent: (event) => setCursor(bucketAt(timeline, event.at)),
                                                labelledBy: titleId,
                                                describedBy: summaryId,
                                                valueText: formatTimestamp(new Date(cursorTime * 1000), {
                                                    seconds: false,
                                                }),
                                                legends: legends(t, timeline, comparing),
                                            }}
                                        />
                                    </Suspense>
                                </ErrorBoundary>
                            </div>
                        )}
                        <div className="flex flex-wrap items-center gap-2 border-t border-line pt-2.5">
                            <span className="text-eyebrow tracking-label text-ink-3 uppercase">
                                {s.addLanes}
                            </span>
                            {EXTRA_LANES.map((lane) => (
                                <Button
                                    key={lane}
                                    size="sm"
                                    tone="ghost"
                                    icon={extra.includes(lane) ? undefined : Plus}
                                    aria-pressed={extra.includes(lane)}
                                    className="border-dashed border-line-strong aria-pressed:border-solid aria-pressed:bg-surface-2"
                                    onClick={() => toggleLane(lane)}
                                >
                                    {s.extra[lane]}
                                </Button>
                            ))}
                            <span className="text-small text-ink-3 md:ml-auto max-md:hidden">{s.hint}</span>
                        </div>
                        {timeline !== null && measured && (
                            <div id={summaryId} className="flex flex-col gap-1 text-small text-ink-3">
                                <p>
                                    {summary(
                                        t,
                                        format,
                                        timeline,
                                        growth.data?.private?.perHour ?? null,
                                        periods,
                                        range,
                                    )}
                                </p>
                                {timeline.frames?.since == null ? (
                                    <p>{s.noPercentiles}</p>
                                ) : timeline.frames.since > timeline.from ? (
                                    <p>
                                        {s.percentilesSince(
                                            formatTimestamp(new Date(timeline.frames.since * 1000), {
                                                seconds: false,
                                            }),
                                        )}
                                    </p>
                                ) : null}
                                <p className="max-md:hidden">{s.keyboard}</p>
                            </div>
                        )}
                        {timeline !== null && measured && (
                            <div>
                                <Button
                                    size="sm"
                                    tone="ghost"
                                    aria-expanded={table}
                                    onClick={() => setTable((shown) => !shown)}
                                >
                                    {table ? s.hideTable : s.showTable}
                                </Button>
                                {table && <WindowTable from={window.from} to={window.to} />}
                            </div>
                        )}
                    </div>
                </section>
                {timeline !== null && measured && (
                    <InstantPanel
                        timeline={timeline}
                        bucket={bucket}
                        hovered={cursor !== null}
                        lanes={lanes}
                        around={around}
                        links={links}
                        events={list}
                        cores={latest?.cores ?? null}
                        words={words}
                    />
                )}
            </div>
            <EventsCard
                events={list}
                omitted={events.data?.omitted ?? []}
                words={words}
                failed={events.isError}
            />
        </div>
    );
}

function Empty({ text }: { text: string }) {
    return (
        <div className="grid h-[320px] place-items-center rounded-card border border-dashed border-line-strong bg-bg-subtle px-6 text-center text-ui text-ink-3">
            {text}
        </div>
    );
}

type Translate = ReturnType<typeof useI18n>['t'];
type Formats = ReturnType<typeof useI18n>['format'];

/** Each lane's title and keys, the swatches drawn as the series are. */
const legends = (t: Translate, timeline: Timeline, comparing: boolean): TimelineChartProps['legends'] => {
    const k = t.statistics.timeline.keys;
    const lanes = t.statistics.timeline.lanes;
    const lineKey = 'h-0.5 w-3 rounded-pill bg-accent';
    const week = { label: k.lastWeek, swatch: 'w-3 border-t-[1.5px] border-dashed border-ink-3' };
    const withWeek = (keys: { label: string; swatch: string }[]) => (comparing ? [...keys, week] : keys);
    const median = frameLine(timeline).kind === 'median';
    return {
        players: {
            title: lanes.players,
            keys: withWeek([
                { label: k.online, swatch: lineKey },
                { label: k.usual, swatch: 'h-2 w-3 rounded-[2px] bg-ink/10' },
            ]),
        },
        frames: {
            title: lanes.frames,
            keys: withWeek([
                { label: median ? k.median : k.mean, swatch: lineKey },
                ...(median ? [{ label: k.tail, swatch: 'h-2 w-3 rounded-[2px] bg-accent/30' }] : []),
                { label: k.budget('50'), swatch: 'w-3 border-t-[1.5px] border-dashed border-warning-fill' },
                { label: k.hitch, swatch: 'h-2 w-1 rounded-[1px] bg-warning-fill' },
            ]),
        },
        cpu: {
            title: lanes.cpu,
            keys: comparing
                ? [{ label: k.total, swatch: lineKey }, week]
                : [
                      { label: k.main, swatch: 'size-2 rounded-[2px] bg-accent/60' },
                      { label: k.server, swatch: 'size-2 rounded-[2px] bg-ink-2/40' },
                      { label: k.other, swatch: 'size-2 rounded-[2px] bg-chart-muted' },
                  ],
        },
        memory: {
            title: lanes.memory,
            keys: withWeek([
                { label: k.inUse, swatch: lineKey },
                { label: k.forecast, swatch: 'h-2 w-3 rounded-[2px] bg-accent/20' },
            ]),
        },
        ping: {
            title: lanes.ping,
            keys: [
                { label: k.average, swatch: lineKey },
                { label: k.p95, swatch: 'h-px w-3 bg-ink-2' },
            ],
        },
        network: {
            title: lanes.network,
            keys: [
                { label: k.received, swatch: lineKey },
                { label: k.sent, swatch: 'h-px w-3 bg-ink-2' },
            ],
        },
        heap: {
            title: lanes.heap,
            keys: [
                { label: k.used, swatch: lineKey },
                { label: k.heapTotal, swatch: 'w-3 border-t border-dashed border-ink-3' },
            ],
        },
        disk: { title: lanes.disk, keys: [{ label: k.latency, swatch: lineKey }] },
    };
};

const measuredValues = (values: readonly (number | null)[]) => values.filter((value) => value !== null);

/** The timeline in words, lane by lane: its text alternative. */
const summary = (
    t: Translate,
    format: Formats,
    timeline: Timeline,
    perHour: number | null,
    periods: ReturnType<typeof downPeriods>,
    range: Range,
) => {
    const s = t.statistics.timeline.summary;
    const parts: string[] = [];
    const players = valuesOf(timeline, 'players');
    const shown = measuredValues(players);
    if (shown.length > 0) {
        const peak = Math.max(...shown);
        const at = timeOf(timeline, players.indexOf(peak));
        parts.push(
            s.players(
                format.number(Math.round(shown.reduce((sum, value) => sum + value, 0) / shown.length)),
                format.number(Math.round(peak)),
                formatTimestamp(new Date(at * 1000), { seconds: false }),
            ),
        );
    }
    const line = frameLine(timeline);
    const values = measuredValues(line.rows.map((row) => row.value));
    if (values.length > 0) {
        const sorted = values.toSorted((a, b) => a - b);
        const middle = sorted[Math.floor(sorted.length / 2)] ?? 0;
        const p99 = measuredValues(timeline.frames?.p99 ?? []);
        parts.push(
            line.kind === 'median' && p99.length > 0
                ? s.frames(format.milliseconds(middle), format.milliseconds(Math.max(...p99)))
                : s.mean(format.milliseconds(middle)),
        );
    }
    const hitchCount = hitches(timeline).reduce((sum, hitch) => sum + hitch.count, 0);
    if (hitchCount > 0) {
        parts.push(s.hitches(hitchCount));
    }
    const cpu = measuredValues(valuesOf(timeline, 'fxsCpu'));
    if (cpu.length > 0) {
        parts.push(
            s.cpu(
                format.percent(cpu.reduce((sum, value) => sum + value, 0) / cpu.length),
                format.percent(Math.max(...cpu)),
            ),
        );
    }
    const memory = measuredValues(valuesOf(timeline, 'fxsPrivate'));
    const newest = memory.at(-1);
    if (newest !== undefined) {
        parts.push(
            s.memory(format.bytes(newest)) +
                (perHour === null ? '' : s.trend(format.bytes(perHour, { signed: true }))),
        );
    }
    if (periods.length > 0) {
        parts.push(t.charts.downFor(format.duration(downTime(periods)), t.charts.ranges[range]));
    }
    return parts.join(' ');
};

const stamp = (seconds: number) => formatTimestamp(new Date(seconds * 1000), { seconds: false });

/** The window again in 48 steps, as a table: the timeline for who does not read charts. */
function WindowTable({ from, to }: { from: number; to: number }) {
    const { t, format } = useI18n();
    const s = t.statistics.timeline.table;
    const query = useQuery(timelineQuery(from, to, TABLE_SERIES, TABLE_POINTS));
    const data = query.data;
    if (data === undefined) {
        return query.isError ? (
            <p className="mt-2 text-small text-ink-3">{t.statistics.timeline.failed}</p>
        ) : null;
    }
    const cell = (value: number | null | undefined, write: (value: number) => string) =>
        value === null || value === undefined ? t.statistics.instant.unknown : write(value);
    const players = valuesOf(data, 'players');
    const cpu = valuesOf(data, 'fxsCpu');
    const memory = valuesOf(data, 'fxsPrivate');
    const mean = valuesOf(data, 'mainTick');
    return (
        <div className="mt-2 max-h-[420px] overflow-auto rounded-box border border-line">
            <table className="w-full text-left text-small">
                <caption className="sr-only">{s.caption(stamp(from), stamp(to))}</caption>
                <thead className="sticky top-0 bg-surface-2 text-ink-3">
                    <tr>
                        {[s.time, s.players, s.median, s.p99, s.cpu, s.memory].map((heading) => (
                            <th key={heading} scope="col" className="px-3 py-2 font-medium">
                                {heading}
                            </th>
                        ))}
                    </tr>
                </thead>
                <tbody>
                    {Array.from({ length: data.buckets }, (_, index) => (
                        <tr key={index} className="border-t border-line">
                            <th scope="row" className="px-3 py-1.5 font-mono font-normal text-ink-2">
                                {stamp(timeOf(data, index))}
                            </th>
                            <td className="px-3 py-1.5">
                                {cell(players[index], (value) => format.number(Math.round(value)))}
                            </td>
                            <td className="px-3 py-1.5">
                                {cell(data.frames?.p50[index] ?? mean[index], (value) =>
                                    format.milliseconds(value),
                                )}
                            </td>
                            <td className="px-3 py-1.5">
                                {cell(data.frames?.p99[index], (value) => format.milliseconds(value))}
                            </td>
                            <td className="px-3 py-1.5">
                                {cell(cpu[index], (value) => format.percent(value))}
                            </td>
                            <td className="px-3 py-1.5">
                                {cell(memory[index], (value) => format.bytes(value))}
                            </td>
                        </tr>
                    ))}
                </tbody>
            </table>
        </div>
    );
}

/** What happened over the window, newest first: the strip's events in words. */
function EventsCard({
    events,
    omitted,
    words,
    failed,
}: {
    events: readonly Parameters<typeof EventLine>[0]['event'][];
    omitted: readonly ('changes' | 'drops')[];
    words: EventWordings;
    failed: boolean;
}) {
    const { t } = useI18n();
    const e = t.statistics.events;
    const newest = events.toReversed();
    const titleId = useId();
    return (
        <section className="card" aria-labelledby={titleId}>
            <div className="card-head">
                <h2 id={titleId} className="card-title">
                    {e.title}
                </h2>
            </div>
            <div className="card-body flex flex-col gap-3 max-md:p-4">
                {failed ? (
                    <p className="text-small text-ink-3">{t.statistics.timeline.failed}</p>
                ) : newest.length === 0 ? (
                    <p className="text-small text-ink-3">{e.none}</p>
                ) : (
                    <ul className="flex flex-col gap-2">
                        {newest.slice(0, LISTED).map((event) => (
                            <EventLine key={event.key} event={event} words={words} />
                        ))}
                        {newest.length > LISTED && (
                            <li className="text-small text-ink-3">{e.more(newest.length - LISTED)}</li>
                        )}
                    </ul>
                )}
                {omitted.map((source) => (
                    <p key={source} className="text-small text-ink-3">
                        {e.omitted[source]}
                    </p>
                ))}
            </div>
        </section>
    );
}
