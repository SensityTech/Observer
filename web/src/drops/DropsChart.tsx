import { keepPreviousData, useQuery } from '@tanstack/react-query';
import {
    AreaSeries,
    HistogramSeries,
    LineSeries,
    LineStyle,
    createChart,
    type ChartOptions,
    type DeepPartial,
    type IChartApi,
    type ISeriesApi,
    type MouseEventParams,
    type Time,
} from 'lightweight-charts';
import { ChartSpline } from 'lucide-react';
import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';

import { seriesQuery } from '../api/drops';
import type { ChartCategory } from '../api/generated/ChartCategory';
import type { DropSeries } from '../api/generated/DropSeries';
import type { DropsRange } from '../api/generated/DropsRange';
import type { Period } from '../charts/coverage';
import { MarkerBand, MarkerLines, markerWords, type PlacedMarker } from '../charts/MarkerOverlay';
import type { ChartMarker } from '../charts/markers';
import { ATTRIBUTION } from '../charts/PlayersChart';
import { DownBands, instantX } from '../charts/primitives';
import { chartOptions, tokenColor, withAlpha } from '../charts/theme';
import { Segmented } from '../components/Segmented';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import {
    baselinePoints,
    baselineSpan,
    countPoints,
    crashPeriods,
    dropBuckets,
    peakOf,
    ratePoints,
    rateTop,
    seriesEnd,
    type DropBucket,
} from './points';
import { CHART_CATEGORIES } from './search';
import { bucketWords, daily, instantWords, pointOf, stampOf, tickWords } from './times';
import { formatRate } from './words';

/** The drop counts' own scale, laid over the lower quarter of the plot, without an axis. */
const COUNTS = 'drops';

/**
 * The theme's options, the scale reading rates rather than whole players, every time worded in the
 * server's zone: the ticks, and the crosshair's label. A chart of days has no time of day.
 */
const dropsOptions = (
    host: Element,
    locale: string,
    zone: string,
    step: number,
): DeepPartial<ChartOptions> => {
    const base = chartOptions(host, locale);
    return {
        ...base,
        timeScale: {
            ...base.timeScale,
            timeVisible: !daily(step),
            tickMarkFormatter: tickWords(locale, zone),
        },
        localization: {
            ...base.localization,
            priceFormatter: (rate: number) => formatRate(locale, rate),
            timeFormatter: (time: Time) => stampOf(time, locale, zone),
        },
    };
};

/** The chart's markers: the runs' moments and FXServer's build changes, in time order. */
const markersOf = (series: DropSeries): ChartMarker[] =>
    [
        ...series.markers,
        ...series.changes.map((change) => ({ kind: 'build' as const, at: change.at, build: change.build })),
    ].toSorted((a, b) => a.at - b.at);

/**
 * The rate of the chosen drops per 100 player-hours with its 4-week baseline and the drops counted,
 * the server's crashes and restarts marked, gaps where nobody played; times in the server's zone.
 */
export function DropsChart({
    range,
    category,
    onCategory,
    rate,
    zone,
}: {
    range: DropsRange;
    category: ChartCategory;
    onCategory: (category: ChartCategory) => void;
    /** The range's rate of these drops, from the summary, for the text alternative. */
    rate: number | null;
    /** The schedule's zone, which every time of the chart reads in. */
    zone: string;
}) {
    const { t, locale, format } = useI18n();
    const words = t.drops.chart;
    const titleId = useId();
    const query = useQuery({ ...seriesQuery(range, category), placeholderData: keepPreviousData });
    const series = query.data ?? null;
    const rows = useMemo(() => (series === null ? [] : dropBuckets(series, zone)), [series, zone]);
    const markers = useMemo(() => (series === null ? [] : markersOf(series)), [series]);
    const periods = useMemo(
        () =>
            series === null
                ? []
                : crashPeriods(series.markers, series.starts[0] ?? series.from, seriesEnd(series)),
        [series],
    );
    const played = rows.filter((row) => row.rate !== null);
    const stale = query.isPlaceholderData;

    let summary: string | null = null;
    if (series !== null && played.length > 0) {
        const drops = rows.reduce((sum, row) => sum + row.drops, 0);
        const peak = peakOf(rows);
        const baseline = baselineSpan(rows);
        const parts = [
            words.summary(
                drops,
                words.what[series.category],
                t.drops.over[series.range],
                rate === null ? null : formatRate(locale, rate),
            ),
            peak === null || peak.rate === null
                ? null
                : words.peak(
                      formatRate(locale, peak.rate),
                      bucketWords(peak.time, series.step, locale, zone),
                  ),
            baseline === null
                ? words.noBaseline
                : words.baselineSpan(formatRate(locale, baseline.low), formatRate(locale, baseline.high)),
            played.length < rows.length ? words.unplayed(rows.length - played.length, rows.length) : null,
        ];
        summary = parts.filter((part) => part !== null).join(' ');
    }
    const said =
        series === null
            ? null
            : markerWords(t, markers, series.step, (seconds) => instantWords(seconds, locale, zone));
    const legend = <Legend down={periods.length > 0} events={markers.length > 0} />;

    return (
        <section className="card flex flex-col" aria-labelledby={titleId}>
            <div className="card-head">
                <span
                    className="grid size-6 place-items-center rounded-[6px] bg-accent text-on-accent"
                    aria-hidden="true"
                >
                    <ChartSpline size={14} strokeWidth={1.75} />
                </span>
                <h2 id={titleId} className="card-title">
                    {words.title}
                </h2>
                {series !== null && (
                    <span className="text-meta text-ink-3">{words.every(format.duration(series.step))}</span>
                )}
                <Segmented
                    label={words.category}
                    choices={CHART_CATEGORIES}
                    value={category}
                    words={words.categories}
                    onChange={onCategory}
                    className="ml-auto md:-my-1.5"
                />
            </div>
            <div className="border-b border-line px-5 py-2.5 max-md:hidden">{legend}</div>
            <div className="card-body flex flex-1 flex-col max-md:p-4">
                {query.isPending ? (
                    <div
                        className="skeleton h-[284px] w-full max-md:h-[224px]"
                        aria-busy="true"
                        aria-label={words.loading}
                    />
                ) : series === null ? (
                    // The page's main figure: its failure is said once, politely.
                    <Empty text={words.failed} live />
                ) : played.length === 0 ? (
                    <Empty text={words.empty} />
                ) : (
                    <figure
                        className={cn('flex flex-1 flex-col transition-opacity', stale && 'opacity-50')}
                        aria-busy={stale}
                    >
                        <Plot
                            // A chart of days holds calendar days, one of hours instants: built anew.
                            key={daily(series.step) ? 'days' : 'hours'}
                            series={series}
                            rows={rows}
                            markers={markers}
                            periods={periods}
                            zone={zone}
                        />
                        <div className="mt-3 md:hidden">{legend}</div>
                        {summary !== null && (
                            <figcaption className="mt-4 border-t border-line pt-3 text-ui text-ink-3">
                                {summary}
                                {said !== null && <span className="mt-1.5 block text-small">{said}</span>}
                            </figcaption>
                        )}
                    </figure>
                )}
            </div>
            <footer className="flex flex-wrap items-center gap-x-1 gap-y-1 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3 max-md:pl-4">
                <span className="mr-auto">{t.drops.rates}</span>
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

/** The keys of the chart, each with its word: above the plot, under it on a phone. */
function Legend({ down, events }: { down: boolean; events: boolean }) {
    const { t } = useI18n();
    const words = t.drops.chart;
    return (
        <div className="flex flex-wrap gap-x-4 gap-y-1.5 text-small text-ink-2" aria-hidden="true">
            <span className="flex items-center gap-1.5">
                <span className="h-0.5 w-3.5 rounded-pill bg-accent" />
                {words.rate}
            </span>
            <span className="flex items-center gap-1.5">
                <span className="w-3.5 border-t-[1.5px] border-dashed border-ink-3" />
                {words.baseline}
            </span>
            <span className="flex items-center gap-1.5">
                <span className="h-3 w-[9px] rounded-[2px] bg-chart-muted" />
                {words.drops}
            </span>
            {down && (
                <span className="flex items-center gap-1.5">
                    <span className="h-3 w-[9px] border-x border-danger bg-danger-soft" />
                    {t.charts.down}
                </span>
            )}
            {events && (
                <span className="flex items-center gap-1.5">
                    <span className="h-3 border-l border-dashed border-ink-3" />
                    {t.charts.event}
                </span>
            )}
        </div>
    );
}

function Empty({ text, live = false }: { text: string; live?: boolean }) {
    return (
        <div
            role={live ? 'status' : undefined}
            className="grid h-[284px] place-items-center rounded-card border border-dashed border-line-strong bg-bg-subtle px-6 text-center text-ui text-ink-3 max-md:h-[224px]"
        >
            {text}
        </div>
    );
}

type Hovered = { x: number; row: DropBucket };

function Plot({
    series,
    rows,
    markers,
    periods,
    zone,
}: {
    series: DropSeries;
    rows: DropBucket[];
    markers: ChartMarker[];
    periods: Period[];
    zone: string;
}) {
    const { t, locale } = useI18n();
    const words = t.drops.chart;
    const host = useRef<HTMLDivElement>(null);
    const chart = useRef<IChartApi | null>(null);
    const rate = useRef<ISeriesApi<'Area'> | null>(null);
    const baseline = useRef<ISeriesApi<'Line'> | null>(null);
    const counts = useRef<ISeriesApi<'Histogram'> | null>(null);
    const optionsOf = useRef({ locale, zone, step: series.step });
    const top = rateTop(rows);
    const topRef = useRef(top);
    const shown = useRef({ rows, periods, starts: series.starts, step: series.step, zone });
    const [down] = useState(() => new DownBands());
    const [placed, setPlaced] = useState<PlacedMarker[]>([]);
    const [hovered, setHovered] = useState<Hovered | null>(null);
    const tip = useRef<HTMLSpanElement>(null);

    /** A bucket's start as this chart holds it: an instant, or a day of the zone. */
    const timeOf = useCallback((seconds: number) => {
        const { step, zone: where } = shown.current;
        return pointOf(seconds, step, where);
    }, []);

    const paintBands = useCallback(() => {
        const element = host.current;
        if (element === null) {
            return;
        }
        const { periods: stretches, starts, step } = shown.current;
        down.set(
            stretches,
            step,
            { fill: tokenColor(element, '--danger-soft'), edge: tokenColor(element, '--danger') },
            starts,
            timeOf,
        );
    }, [down, timeOf]);

    // A marker stands at its instant, between its bucket's start and the next one's.
    const place = useCallback(() => {
        const api = chart.current;
        if (api === null) {
            return;
        }
        const scale = api.priceScale('left').width();
        const { starts, step } = shown.current;
        setPlaced(
            markers.flatMap((marker) => {
                const x = instantX(api.timeScale(), starts, step, marker.at, timeOf);
                return x === null ? [] : [{ marker, x: x + scale }];
            }),
        );
    }, [markers, timeOf]);

    useEffect(() => {
        topRef.current = top;
    }, [top]);

    // Built once: the series and their scales never change shape, only their data and colours.
    useEffect(() => {
        const element = host.current;
        if (element === null) {
            return;
        }
        const options = () => {
            const { locale: language, zone: where, step } = optionsOf.current;
            return dropsOptions(element, language, where, step);
        };
        const api = createChart(element, options());
        const autoscale = () => ({ priceRange: { minValue: 0, maxValue: topRef.current } });
        // Drawn first, so that the rate and its baseline stay over the bars.
        counts.current = api.addSeries(HistogramSeries, {
            priceScaleId: COUNTS,
            priceLineVisible: false,
            lastValueVisible: false,
        });
        rate.current = api.addSeries(AreaSeries, {
            priceScaleId: 'left',
            lineWidth: 2,
            priceLineVisible: false,
            lastValueVisible: false,
            crosshairMarkerRadius: 4,
            autoscaleInfoProvider: autoscale,
        });
        baseline.current = api.addSeries(LineSeries, {
            priceScaleId: 'left',
            lineWidth: 1,
            lineStyle: LineStyle.Dashed,
            priceLineVisible: false,
            lastValueVisible: false,
            crosshairMarkerVisible: false,
            autoscaleInfoProvider: autoscale,
        });
        api.priceScale(COUNTS).applyOptions({ scaleMargins: { top: 0.75, bottom: 0 } });
        rate.current.attachPrimitive(down);
        chart.current = api;
        const paint = () => {
            const accent = tokenColor(element, '--accent');
            api.applyOptions(options());
            rate.current?.applyOptions({
                lineColor: accent,
                topColor: withAlpha(accent, 0.1),
                bottomColor: withAlpha(accent, 0.1),
                crosshairMarkerBorderColor: tokenColor(element, '--surface'),
                crosshairMarkerBackgroundColor: accent,
            });
            baseline.current?.applyOptions({ color: tokenColor(element, '--ink-3') });
            counts.current?.applyOptions({ color: tokenColor(element, '--chart-muted') });
            paintBands();
        };
        paint();
        const theme = new MutationObserver(paint);
        theme.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
        const hover = (event: MouseEventParams<Time>) => {
            const row = shown.current.rows.find((each) => each.point === event.time);
            if (event.point === undefined || row === undefined) {
                setHovered(null);
                return;
            }
            setHovered({ x: event.point.x + api.priceScale('left').width(), row });
        };
        api.subscribeCrosshairMove(hover);
        return () => {
            theme.disconnect();
            api.unsubscribeCrosshairMove(hover);
            api.remove();
            chart.current = null;
            rate.current = null;
            baseline.current = null;
            counts.current = null;
        };
    }, [down, paintBands]);

    useEffect(() => {
        optionsOf.current = { locale, zone, step: series.step };
        const element = host.current;
        if (element !== null) {
            chart.current?.applyOptions(dropsOptions(element, locale, zone, series.step));
        }
    }, [locale, zone, series.step]);

    // A page read anew is drawn whole: every 30 s at most, never point by point.
    useEffect(() => {
        const element = host.current;
        shown.current = { rows, periods, starts: series.starts, step: series.step, zone };
        if (element === null) {
            return;
        }
        const transparent = withAlpha(tokenColor(element, '--accent'), 0);
        rate.current?.setData(ratePoints(rows, transparent));
        baseline.current?.setData(baselinePoints(rows, withAlpha(tokenColor(element, '--ink-3'), 0)));
        counts.current?.setData(countPoints(rows));
        chart.current?.timeScale().fitContent();
        paintBands();
        place();
    }, [rows, periods, series.starts, series.step, zone, paintBands, place]);

    useEffect(() => {
        const api = chart.current;
        const element = host.current;
        if (api === null || element === null) {
            return;
        }
        const timeScale = api.timeScale();
        timeScale.subscribeVisibleLogicalRangeChange(place);
        const resized = new ResizeObserver(place);
        resized.observe(element);
        return () => {
            timeScale.unsubscribeVisibleLogicalRangeChange(place);
            resized.disconnect();
        };
    }, [place]);

    // Centred on the pointer, but never past either side of the plot.
    useLayoutEffect(() => {
        const element = tip.current;
        const plot = host.current;
        if (hovered === null || element === null || plot === null) {
            return;
        }
        const half = element.offsetWidth / 2;
        element.style.left = `${Math.max(half, Math.min(hovered.x, plot.clientWidth - half))}px`;
    }, [hovered]);

    return (
        <div className="relative flex flex-1 flex-col" aria-hidden="true">
            <MarkerBand markers={placed} zone={zone} />
            <div className="relative h-[260px] max-md:h-[200px]">
                <div ref={host} className="absolute inset-0" />
                <div className="pointer-events-none absolute inset-0">
                    <MarkerLines markers={placed} />
                    {hovered && (
                        <span
                            ref={tip}
                            className="absolute top-2 flex -translate-x-1/2 flex-col rounded-sm border border-line-strong bg-surface px-2.5 py-1.5 whitespace-nowrap shadow-pop"
                        >
                            <span className="text-ui font-semibold text-ink">
                                {hovered.row.rate === null
                                    ? words.tooltipNobody
                                    : words.tooltipRate(formatRate(locale, hovered.row.rate))}
                            </span>
                            <span className="text-small text-ink-2">
                                {[
                                    words.tooltipDrops(hovered.row.drops),
                                    hovered.row.baseline === null
                                        ? null
                                        : words.tooltipBaseline(formatRate(locale, hovered.row.baseline)),
                                ]
                                    .filter((part) => part !== null)
                                    .join(' · ')}
                            </span>
                            <span className="font-mono text-meta text-ink-3">
                                {stampOf(hovered.row.point, locale, zone)}
                            </span>
                        </span>
                    )}
                </div>
            </div>
        </div>
    );
}
