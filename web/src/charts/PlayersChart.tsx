import { keepPreviousData, useQuery } from '@tanstack/react-query';
import {
    AreaSeries,
    LineSeries,
    LineStyle,
    LineType,
    createChart,
    type IChartApi,
    type ISeriesApi,
    type MouseEventParams,
    type Time,
    type UTCTimestamp,
} from 'lightweight-charts';
import { Users } from 'lucide-react';
import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';

import type { MetricsSeries } from '../api/generated/MetricsSeries';
import type { Range } from '../api/generated/Range';
import { seriesQuery } from '../api/metrics';
import { useI18n } from '../i18n/I18nProvider';
import { formatTimestamp } from '../i18n/format';
import { cn } from '../lib/cn';
import { useLiveMetrics } from '../metrics/live';
import { mergeLive } from '../metrics/merge';
import { downPeriods, downTime, type Period } from './coverage';
import { MarkerBand, MarkerLines, markerWords, type PlacedMarker } from './MarkerOverlay';
import { bucketOf, chartTime } from './markers';
import { buckets, playerPoints, resumes, slotPoints, type Bucket } from './points';
import { DownBands, RangeBand, type Extremes } from './primitives';
import { RangeControl } from './RangeControl';
import { scaleTop } from './scale';
import { chartOptions, tokenColor, withAlpha } from './theme';

const SERIES = ['players', 'slots'] as const;
const ATTRIBUTION = 'https://www.tradingview.com/';
/** Players and slots are whole: the scale steps and reads in whole numbers. */
const WHOLE = { type: 'price', precision: 0, minMove: 1 } as const;
/** The min–max band's fill: the accent, lighter than the area under the line. */
const BAND_ALPHA = 0.12;

/** The players' lowest and highest of each bucket, which a range of one sample per bucket lacks. */
const extremes = (page: MetricsSeries): Extremes[] => {
    const players = page.series.find((data) => data.series === 'players');
    if (players?.min === undefined || players.max === undefined) {
        return [];
    }
    const { min, max } = players;
    return Array.from({ length: page.buckets }, (_, index) => ({
        time: page.from + index * page.step,
        min: min[index] ?? null,
        max: max[index] ?? null,
    }));
};

type Overlay = {
    markers: PlacedMarker[];
    peak: { x: number; y: number } | null;
    /** The slots label, just right of the price scale, whose width depends on its labels. */
    slots: { x: number; y: number; value: number } | null;
};

export default function PlayersChart() {
    const { t, format } = useI18n();
    const [range, setRange] = useState<Range>('24h');
    const query = useQuery({ ...seriesQuery(range, SERIES), placeholderData: keepPreviousData });
    const { recent, latest } = useLiveMetrics();
    const page = useMemo(() => (query.data ? mergeLive(query.data, recent) : null), [query.data, recent]);
    const rows = useMemo(() => (page ? buckets(page) : []), [page]);
    const band = useMemo(() => (page ? extremes(page) : []), [page]);
    const periods = useMemo(() => (page ? downPeriods(page) : []), [page]);
    const measured = rows.filter((row) => row.players !== null);
    const banded = band.some((bucket) => bucket.min !== null && bucket.max !== null);
    const titleId = useId();
    const stale = query.isPlaceholderData;

    const peak = measured.reduce<Bucket | null>(
        (best, row) => (best === null || (row.players ?? 0) > (best.players ?? 0) ? row : best),
        null,
    );
    const top = scaleTop(rows);

    let summary: string | null = null;
    if (page !== null && peak !== null && peak.players !== null) {
        const average = measured.reduce((sum, row) => sum + (row.players ?? 0), 0) / measured.length;
        const newest = latest?.values.players ?? null;
        const figures = [
            format.number(Math.round(average)),
            t.charts.ranges[range],
            format.number(Math.round(peak.players)),
            chartTime(peak.time, page.step),
        ] as const;
        summary =
            newest === null
                ? t.charts.players.summary(...figures)
                : t.charts.players.summaryNow(format.number(newest), ...figures);
        if (periods.length > 0) {
            summary = `${summary} ${t.charts.downFor(format.duration(downTime(periods)), t.charts.ranges[range])}`;
        }
    }
    const words = page ? markerWords(t, page.markers, page.step) : null;

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
                    {t.charts.players.title}
                </h2>
                {page && (
                    <span className="text-meta text-ink-3">
                        {t.charts.players.every(format.duration(page.step))}
                    </span>
                )}
                <RangeControl label={t.charts.players.range} value={range} onChange={setRange} />
            </div>
            <div
                className="flex flex-wrap gap-x-4 gap-y-1.5 border-b border-line px-5 py-2.5 text-small text-ink-2 max-md:px-4"
                aria-hidden="true"
            >
                <span className="flex items-center gap-1.5">
                    <span className="h-0.5 w-3.5 rounded-pill bg-accent" />
                    {t.charts.players.players}
                </span>
                <span className="flex items-center gap-1.5">
                    <span className="w-3.5 border-t-[1.5px] border-dashed border-ink-3" />
                    {t.charts.players.maxSlots}
                </span>
                {banded && (
                    <span className="flex items-center gap-1.5">
                        <span className="h-2.5 w-3.5 rounded-[2px] bg-accent/12" />
                        {t.charts.players.minMax}
                    </span>
                )}
                {periods.length > 0 && (
                    <span className="flex items-center gap-1.5">
                        <span className="h-3 w-[9px] border-x border-danger bg-danger-soft" />
                        {t.charts.down}
                    </span>
                )}
                <span className="flex items-center gap-1.5">
                    <span className="h-3 border-l border-dashed border-ink-3" />
                    {t.charts.event}
                </span>
            </div>
            <div className="card-body flex flex-1 flex-col max-md:p-4">
                {query.isPending ? (
                    <div
                        className="skeleton h-[284px] w-full max-md:h-[224px]"
                        aria-busy="true"
                        aria-label={t.charts.players.loading}
                    />
                ) : page === null ? (
                    <Empty text={t.charts.players.failed} />
                ) : measured.length === 0 ? (
                    <Empty text={t.charts.players.empty} />
                ) : (
                    <figure
                        className={cn('flex flex-1 flex-col transition-opacity', stale && 'opacity-50')}
                        aria-busy={stale}
                    >
                        <Plot
                            page={page}
                            fetched={query.data ?? null}
                            rows={rows}
                            band={band}
                            periods={periods}
                            top={top}
                            peakTime={peak?.time ?? null}
                        />
                        {summary !== null && (
                            <figcaption className="mt-4 border-t border-line pt-3 text-ui text-ink-3">
                                {summary}
                                {words !== null && <span className="mt-1.5 block text-small">{words}</span>}
                            </figcaption>
                        )}
                    </figure>
                )}
            </div>
            <footer className="flex flex-wrap items-center gap-1 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3">
                {t.charts.attribution}{' '}
                <a
                    href={ATTRIBUTION}
                    target="_blank"
                    rel="noreferrer"
                    className="underline-offset-2 hover:text-ink-2 hover:underline"
                >
                    {t.charts.library}
                </a>
            </footer>
        </section>
    );
}

function Empty({ text }: { text: string }) {
    return (
        <div className="grid h-[284px] place-items-center rounded-card border border-dashed border-line-strong bg-bg-subtle px-6 text-center text-ui text-ink-3 max-md:h-[224px]">
            {text}
        </div>
    );
}

function Plot({
    page,
    fetched,
    rows,
    band,
    periods,
    top,
    peakTime,
}: {
    page: MetricsSeries;
    /** The page as read, before live samples: a new one is drawn anew. */
    fetched: MetricsSeries | null;
    rows: Bucket[];
    band: Extremes[];
    periods: Period[];
    top: number;
    peakTime: number | null;
}) {
    const { t, format, locale } = useI18n();
    const host = useRef<HTMLDivElement>(null);
    const chart = useRef<IChartApi | null>(null);
    const players = useRef<ISeriesApi<'Area'> | null>(null);
    const slots = useRef<ISeriesApi<'Line'> | null>(null);
    const drawn = useRef<MetricsSeries | null>(null);
    const topRef = useRef(top);
    const localeRef = useRef(locale);
    const [overlay, setOverlay] = useState<Overlay>({ markers: [], peak: null, slots: null });
    const [tooltip, setTooltip] = useState<{ x: number; players: number; time: number } | null>(null);
    const tip = useRef<HTMLSpanElement>(null);
    const [primitives] = useState(() => ({ band: new RangeBand(), down: new DownBands() }));
    const shown = useRef({ band, periods, step: page.step });

    // Their colours are resolved here, again on each change of theme.
    const paintPrimitives = useCallback(() => {
        const element = host.current;
        if (element === null) {
            return;
        }
        const { band: points, periods: down, step } = shown.current;
        primitives.band.set(points, withAlpha(tokenColor(element, '--accent'), BAND_ALPHA));
        primitives.down.set(down, step, {
            fill: tokenColor(element, '--danger-soft'),
            edge: tokenColor(element, '--danger'),
        });
    }, [primitives]);

    const place = useCallback(() => {
        const api = chart.current;
        const area = players.current;
        if (api === null || area === null) {
            return;
        }
        const scale = api.priceScale('left').width();
        const x = (time: number) => {
            const at = api.timeScale().timeToCoordinate(time as UTCTimestamp);
            return at === null ? null : at + scale;
        };
        const markers = page.markers.flatMap((marker) => {
            const at = x(bucketOf(marker, page.from, page.step));
            return at === null ? [] : [{ marker, x: at }];
        });
        const peakRow = peakTime === null ? undefined : rows.find((row) => row.time === peakTime);
        const peakX = peakTime === null ? null : x(peakTime);
        const peakY = peakRow?.players == null ? null : area.priceToCoordinate(peakRow.players);
        const latest = rows.findLast((row) => row.slots !== null)?.slots ?? null;
        const slotsY = latest === null ? null : (slots.current?.priceToCoordinate(latest) ?? null);
        setOverlay({
            markers,
            peak: peakX === null || peakY === null ? null : { x: peakX, y: peakY },
            slots: latest === null || slotsY === null ? null : { x: scale + 8, y: slotsY, value: latest },
        });
    }, [page, rows, peakTime]);

    useEffect(() => {
        topRef.current = top;
    }, [top]);

    // Built once: the series and their scale never change shape, only their data and colours.
    useEffect(() => {
        const element = host.current;
        if (element === null) {
            return;
        }
        const api = createChart(element, chartOptions(element, localeRef.current));
        const autoscale = () => ({ priceRange: { minValue: 0, maxValue: topRef.current } });
        players.current = api.addSeries(AreaSeries, {
            priceScaleId: 'left',
            priceFormat: WHOLE,
            lineWidth: 2,
            priceLineVisible: false,
            lastValueVisible: false,
            crosshairMarkerRadius: 4,
            autoscaleInfoProvider: autoscale,
        });
        slots.current = api.addSeries(LineSeries, {
            priceScaleId: 'left',
            priceFormat: WHOLE,
            lineWidth: 1,
            lineStyle: LineStyle.Dashed,
            lineType: LineType.WithSteps,
            priceLineVisible: false,
            lastValueVisible: false,
            crosshairMarkerVisible: false,
            autoscaleInfoProvider: autoscale,
        });
        players.current.attachPrimitive(primitives.band);
        players.current.attachPrimitive(primitives.down);
        chart.current = api;
        const paint = () => {
            const accent = tokenColor(element, '--accent');
            api.applyOptions(chartOptions(element, localeRef.current));
            players.current?.applyOptions({
                lineColor: accent,
                topColor: withAlpha(accent, 0.1),
                bottomColor: withAlpha(accent, 0.1),
                crosshairMarkerBorderColor: tokenColor(element, '--surface'),
                crosshairMarkerBackgroundColor: accent,
            });
            slots.current?.applyOptions({ color: tokenColor(element, '--ink-3') });
            paintPrimitives();
        };
        paint();
        const theme = new MutationObserver(paint);
        theme.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
        const hover = (event: MouseEventParams<Time>) => {
            const point = players.current ? event.seriesData.get(players.current) : undefined;
            if (
                event.time === undefined ||
                event.point === undefined ||
                point === undefined ||
                !('value' in point) ||
                typeof point.value !== 'number'
            ) {
                setTooltip(null);
                return;
            }
            setTooltip({
                x: event.point.x + api.priceScale('left').width(),
                players: point.value,
                time: event.time as number,
            });
        };
        api.subscribeCrosshairMove(hover);
        return () => {
            theme.disconnect();
            api.unsubscribeCrosshairMove(hover);
            api.remove();
            chart.current = null;
            players.current = null;
            slots.current = null;
            drawn.current = null;
        };
    }, [primitives, paintPrimitives]);

    useEffect(() => {
        shown.current = { band, periods, step: page.step };
        paintPrimitives();
    }, [band, periods, page.step, paintPrimitives]);

    useEffect(() => {
        localeRef.current = locale;
        const element = host.current;
        if (element !== null) {
            chart.current?.applyOptions(chartOptions(element, locale));
        }
    }, [locale]);

    // A new page is set at once, and so is the line picking up after a stop; live samples
    // otherwise move the newest bucket only.
    useEffect(() => {
        const element = host.current;
        const area = players.current;
        const line = slots.current;
        if (element === null || area === null || line === null || rows.length === 0) {
            return;
        }
        const transparent = withAlpha(tokenColor(element, '--accent'), 0);
        if (drawn.current === fetched && !resumes(rows)) {
            const newest = rows.slice(-1);
            for (const point of playerPoints(newest, transparent)) {
                area.update(point);
            }
            for (const point of slotPoints(newest)) {
                line.update(point);
            }
        } else {
            area.setData(playerPoints(rows, transparent));
            line.setData(slotPoints(rows));
            if (drawn.current !== fetched) {
                chart.current?.timeScale().fitContent();
            }
        }
        drawn.current = fetched;
        place();
    }, [fetched, rows, place]);

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

    // Centred on the pointer, but never past either side of the plot: at phone width the newest
    // bucket is close enough to the edge to scroll the page.
    useLayoutEffect(() => {
        const element = tip.current;
        const plot = host.current;
        if (tooltip === null || element === null || plot === null) {
            return;
        }
        const half = element.offsetWidth / 2;
        element.style.left = `${Math.max(half, Math.min(tooltip.x, plot.clientWidth - half))}px`;
    }, [tooltip]);

    return (
        <div className="relative flex flex-1 flex-col" aria-hidden="true">
            <MarkerBand markers={overlay.markers} />
            <div className="relative h-[260px] max-md:h-[200px]">
                <div ref={host} className="absolute inset-0" />
                <div className="pointer-events-none absolute inset-0" aria-hidden="true">
                    <MarkerLines markers={overlay.markers} />
                    {overlay.slots && (
                        <span
                            className="absolute font-mono text-micro leading-none text-ink-3"
                            style={{ left: overlay.slots.x, top: overlay.slots.y - 14 }}
                        >
                            {t.charts.players.maxSlotsAt(format.number(overlay.slots.value))}
                        </span>
                    )}
                    {overlay.peak && (
                        <span
                            className="absolute size-[9px] -translate-x-1/2 -translate-y-1/2 rounded-pill bg-accent ring-2 ring-surface"
                            style={{ left: overlay.peak.x, top: overlay.peak.y }}
                        />
                    )}
                    {tooltip && (
                        <span
                            ref={tip}
                            className="absolute top-2 flex -translate-x-1/2 flex-col rounded-sm border border-line-strong bg-surface px-2.5 py-1.5 whitespace-nowrap shadow-pop"
                        >
                            <span className="text-ui font-semibold text-ink">
                                {t.charts.players.tooltip(Math.round(tooltip.players))}
                            </span>
                            <span className="font-mono text-meta text-ink-3">
                                {formatTimestamp(new Date(tooltip.time * 1000), { seconds: false })}
                            </span>
                        </span>
                    )}
                </div>
            </div>
        </div>
    );
}
