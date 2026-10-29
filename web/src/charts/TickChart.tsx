import { keepPreviousData, useQuery } from '@tanstack/react-query';
import {
    AreaSeries,
    LineSeries,
    LineStyle,
    PriceScaleMode,
    createChart,
    type AreaData,
    type IChartApi,
    type IPriceLine,
    type ISeriesApi,
    type LineData,
    type MouseEventParams,
    type Time,
    type UTCTimestamp,
} from 'lightweight-charts';
import { Gauge } from 'lucide-react';
import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';

import type { MetricsSeries } from '../api/generated/MetricsSeries';
import type { Range } from '../api/generated/Range';
import { seriesQuery } from '../api/metrics';
import { useI18n } from '../i18n/I18nProvider';
import { formatNumber, formatTimestamp } from '../i18n/format';
import { cn } from '../lib/cn';
import { useLiveMetrics } from '../metrics/live';
import { mergeLive } from '../metrics/merge';
import { downPeriods, type Period } from './coverage';
import { MarkerBand, MarkerLines, markerWords, type PlacedMarker } from './MarkerOverlay';
import { bucketOf, chartTime } from './markers';
import { gapPoints } from './points';
import { DownBands } from './primitives';
import { RangeControl } from './RangeControl';
import { chartOptions, tokenColor, withAlpha } from './theme';
import { BUDGETS, TICK_SERIES, tickBuckets, tickFigures, tickScale, type TickBucket } from './tick';

type Thread = 'main' | 'network' | 'sync';

/** The chart's options: the shared ones, with milliseconds of up to two decimals on the scale. */
const options = (element: HTMLElement, locale: string) => {
    const base = chartOptions(element, locale);
    return {
        ...base,
        localization: {
            ...base.localization,
            priceFormatter: (ms: number) => formatNumber(locale, ms, { maximumFractionDigits: 2 }),
        },
    };
};

export default function TickChart() {
    const { t, format } = useI18n();
    const [range, setRange] = useState<Range>('6h');
    const query = useQuery({ ...seriesQuery(range, TICK_SERIES), placeholderData: keepPreviousData });
    const { recent } = useLiveMetrics();
    const page = useMemo(() => (query.data ? mergeLive(query.data, recent) : null), [query.data, recent]);
    const rows = useMemo(() => (page ? tickBuckets(page) : []), [page]);
    const periods = useMemo(() => (page ? downPeriods(page) : []), [page]);
    const titleId = useId();
    const stale = query.isPlaceholderData;
    const measured = rows.some((row) => row.main !== null || row.network !== null || row.sync !== null);
    const figures = page ? tickFigures(page) : null;
    const words = page ? markerWords(t, page.markers, page.step) : null;
    const share = (percent: number | null) => (percent === null ? t.tiles.unknown : format.percent(percent));

    return (
        <section className="card flex flex-1 flex-col" aria-labelledby={titleId}>
            <div className="card-head">
                <span
                    className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2"
                    aria-hidden="true"
                >
                    <Gauge size={14} strokeWidth={1.75} />
                </span>
                <h2 id={titleId} className="card-title">
                    {t.charts.tick.title}
                </h2>
                <span className="text-meta text-ink-3">{t.charts.tick.meta}</span>
                <RangeControl value={range} onChange={setRange} />
            </div>
            <div
                className="flex flex-wrap gap-x-4 gap-y-1.5 border-b border-line px-5 py-2.5 text-small text-ink-2 max-md:px-4"
                aria-hidden="true"
            >
                <span className="flex items-center gap-1.5">
                    <span className="h-0.5 w-3.5 rounded-pill bg-accent" />
                    {t.charts.tick.main}
                </span>
                <span className="flex items-center gap-1.5">
                    <span className="h-px w-3.5 bg-ink-2" />
                    {t.charts.tick.network}
                </span>
                <span className="flex items-center gap-1.5">
                    <span className="h-px w-3.5 bg-ink-3" />
                    {t.charts.tick.sync}
                </span>
                <span className="flex items-center gap-1.5">
                    <span className="w-3.5 border-t-[1.5px] border-dashed border-warning-fill" />
                    {t.charts.tick.budget(format.number(BUDGETS.main))}
                </span>
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
                        className="skeleton h-[264px] w-full max-md:h-[224px]"
                        aria-busy="true"
                        aria-label={t.charts.tick.loading}
                    />
                ) : page === null || figures === null ? (
                    <Empty text={t.charts.tick.failed} />
                ) : !measured ? (
                    <Empty text={t.charts.tick.empty} />
                ) : (
                    <figure
                        className={cn('flex flex-1 flex-col transition-opacity', stale && 'opacity-50')}
                        aria-busy={stale}
                    >
                        <Plot page={page} fetched={query.data ?? null} rows={rows} periods={periods} />
                        <dl className="mt-4 flex flex-wrap gap-x-7 gap-y-2.5 border-t border-line pt-3.5">
                            <Figure
                                term={t.charts.tick.average}
                                value={
                                    figures.mainAverage === null
                                        ? t.tiles.unknown
                                        : format.number(figures.mainAverage, {
                                              maximumSignificantDigits: 3,
                                              maximumFractionDigits: 2,
                                          })
                                }
                                unit={figures.mainAverage === null ? undefined : t.charts.tick.ms}
                            />
                            <Figure
                                term={t.charts.tick.mainOver(format.number(BUDGETS.main))}
                                value={share(figures.mainOver)}
                                unit={figures.mainOver === null ? undefined : t.charts.tick.ofTicks}
                            />
                            <Figure
                                term={t.charts.tick.othersOver(
                                    format.number(BUDGETS.network),
                                    format.number(BUDGETS.sync),
                                )}
                                value={t.tiles.pair(share(figures.networkOver), share(figures.syncOver))}
                            />
                            <Figure
                                term={t.charts.tick.longest}
                                value={
                                    figures.hitch === null
                                        ? t.charts.tick.none
                                        : format.number(figures.hitch.ms)
                                }
                                unit={
                                    figures.hitch === null
                                        ? undefined
                                        : t.charts.tick.msAt(chartTime(figures.hitch.at, page.step))
                                }
                            />
                        </dl>
                        {words !== null && <p className="mt-2.5 text-small text-ink-3">{words}</p>}
                    </figure>
                )}
            </div>
        </section>
    );
}

function Figure({ term, value, unit }: { term: string; value: string; unit?: string }) {
    return (
        <div>
            <dt className="text-meta text-ink-3">{term}</dt>
            <dd className="text-figure-sm text-ink">
                {value}
                {unit !== undefined && <span className="ml-1 text-small font-medium text-ink-3">{unit}</span>}
            </dd>
        </div>
    );
}

function Empty({ text }: { text: string }) {
    return (
        <div className="grid h-[264px] place-items-center rounded-card border border-dashed border-line-strong bg-bg-subtle px-6 text-center text-ui text-ink-3 max-md:h-[224px]">
            {text}
        </div>
    );
}

type Overlay = { markers: PlacedMarker[]; budget: { x: number; y: number } | null };
type Tooltip = { x: number; time: number } & Record<Thread, number | null>;

function Plot({
    page,
    fetched,
    rows,
    periods,
}: {
    page: MetricsSeries;
    /** The page as read, before live samples: a new one is drawn anew. */
    fetched: MetricsSeries | null;
    rows: TickBucket[];
    periods: Period[];
}) {
    const { t, format, locale } = useI18n();
    const host = useRef<HTMLDivElement>(null);
    const chart = useRef<IChartApi | null>(null);
    const lines = useRef<{
        main: ISeriesApi<'Area'>;
        network: ISeriesApi<'Line'>;
        sync: ISeriesApi<'Line'>;
    } | null>(null);
    const budget = useRef<IPriceLine | null>(null);
    const [down] = useState(() => new DownBands());
    const drawn = useRef<MetricsSeries | null>(null);
    const scale = useRef(tickScale(rows));
    const localeRef = useRef(locale);
    const shown = useRef({ periods, step: page.step });
    const tip = useRef<HTMLSpanElement>(null);
    const [overlay, setOverlay] = useState<Overlay>({ markers: [], budget: null });
    const [tooltip, setTooltip] = useState<Tooltip | null>(null);

    const paintDown = useCallback(() => {
        const element = host.current;
        if (element !== null) {
            down.set(shown.current.periods, shown.current.step, {
                fill: tokenColor(element, '--danger-soft'),
                edge: tokenColor(element, '--danger'),
            });
        }
    }, [down]);

    const place = useCallback(() => {
        const api = chart.current;
        const main = lines.current?.main;
        if (api === null || main === undefined) {
            return;
        }
        const left = api.priceScale('left').width();
        const markers = page.markers.flatMap((marker) => {
            const at = api
                .timeScale()
                .timeToCoordinate(bucketOf(marker, page.from, page.step) as UTCTimestamp);
            return at === null ? [] : [{ marker, x: at + left }];
        });
        const y = main.priceToCoordinate(BUDGETS.main);
        setOverlay({ markers, budget: y === null ? null : { x: left + 8, y } });
    }, [page]);

    useEffect(() => {
        scale.current = tickScale(rows);
    }, [rows]);

    // Built once: the three series and their scale keep their shape, only data and colours change.
    useEffect(() => {
        const element = host.current;
        if (element === null) {
            return;
        }
        const api = createChart(element, options(element, localeRef.current));
        api.priceScale('left').applyOptions({ mode: PriceScaleMode.Logarithmic });
        const autoscaleInfoProvider = () => ({ priceRange: scale.current });
        const quiet = {
            priceLineVisible: false,
            lastValueVisible: false,
            priceScaleId: 'left',
            autoscaleInfoProvider,
        };
        const main = api.addSeries(AreaSeries, { ...quiet, lineWidth: 2, crosshairMarkerRadius: 4 });
        const network = api.addSeries(LineSeries, { ...quiet, lineWidth: 1, crosshairMarkerVisible: false });
        const sync = api.addSeries(LineSeries, { ...quiet, lineWidth: 1, crosshairMarkerVisible: false });
        budget.current = main.createPriceLine({
            price: BUDGETS.main,
            lineWidth: 1,
            lineStyle: LineStyle.Dashed,
            axisLabelVisible: false,
            title: '',
        });
        main.attachPrimitive(down);
        lines.current = { main, network, sync };
        chart.current = api;
        const paint = () => {
            const accent = tokenColor(element, '--accent');
            api.applyOptions(options(element, localeRef.current));
            main.applyOptions({
                lineColor: accent,
                topColor: withAlpha(accent, 0.1),
                bottomColor: withAlpha(accent, 0.1),
                crosshairMarkerBorderColor: tokenColor(element, '--surface'),
                crosshairMarkerBackgroundColor: accent,
            });
            network.applyOptions({ color: tokenColor(element, '--ink-2') });
            sync.applyOptions({ color: tokenColor(element, '--ink-3') });
            budget.current?.applyOptions({ color: tokenColor(element, '--warning-fill') });
            paintDown();
        };
        paint();
        const theme = new MutationObserver(paint);
        theme.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
        const hover = (event: MouseEventParams<Time>) => {
            const value = (series: ISeriesApi<'Area'> | ISeriesApi<'Line'>) => {
                const point = event.seriesData.get(series);
                return point !== undefined && 'value' in point && typeof point.value === 'number'
                    ? point.value
                    : null;
            };
            const at = { main: value(main), network: value(network), sync: value(sync) };
            if (
                event.time === undefined ||
                event.point === undefined ||
                (at.main ?? at.network ?? at.sync) === null
            ) {
                setTooltip(null);
                return;
            }
            setTooltip({
                x: event.point.x + api.priceScale('left').width(),
                time: event.time as number,
                ...at,
            });
        };
        api.subscribeCrosshairMove(hover);
        return () => {
            theme.disconnect();
            api.unsubscribeCrosshairMove(hover);
            api.remove();
            chart.current = null;
            lines.current = null;
            budget.current = null;
            drawn.current = null;
        };
    }, [down, paintDown]);

    useEffect(() => {
        localeRef.current = locale;
        const element = host.current;
        if (element !== null) {
            chart.current?.applyOptions(options(element, locale));
        }
    }, [locale]);

    useEffect(() => {
        shown.current = { periods, step: page.step };
        paintDown();
    }, [periods, page.step, paintDown]);

    // A new page is set at once, and so is a line picking up after a stop; live samples otherwise
    // move the newest bucket only.
    useEffect(() => {
        const element = host.current;
        const series = lines.current;
        if (element === null || series === null || rows.length === 0) {
            return;
        }
        const transparent = withAlpha(tokenColor(element, '--accent'), 0);
        const of = (thread: Thread) => rows.map((row) => ({ time: row.time, value: row[thread] }));
        const resumed = (['main', 'network', 'sync'] as const).some(
            (thread) => rows.at(-1)?.[thread] != null && rows.at(-2)?.[thread] === null,
        );
        const area = gapPoints<AreaData>(of('main'), {
            lineColor: transparent,
            topColor: transparent,
            bottomColor: transparent,
        });
        const network = gapPoints<LineData>(of('network'), { color: transparent });
        const sync = gapPoints<LineData>(of('sync'), { color: transparent });
        if (drawn.current === fetched && !resumed) {
            const [main, networkPoint, syncPoint] = [area.at(-1), network.at(-1), sync.at(-1)];
            if (main !== undefined) {
                series.main.update(main);
            }
            if (networkPoint !== undefined) {
                series.network.update(networkPoint);
            }
            if (syncPoint !== undefined) {
                series.sync.update(syncPoint);
            }
        } else {
            series.main.setData(area);
            series.network.setData(network);
            series.sync.setData(sync);
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

    // Centred on the pointer, but never past either side of the plot.
    useLayoutEffect(() => {
        const element = tip.current;
        const plot = host.current;
        if (tooltip === null || element === null || plot === null) {
            return;
        }
        const half = element.offsetWidth / 2;
        element.style.left = `${Math.max(half, Math.min(tooltip.x, plot.clientWidth - half))}px`;
    }, [tooltip]);

    const ms = (value: number | null) => (value === null ? t.tiles.unknown : format.milliseconds(value));

    return (
        <div className="relative flex flex-1 flex-col" aria-hidden="true">
            <MarkerBand markers={overlay.markers} />
            <div className="relative h-[240px] max-md:h-[200px]">
                <div ref={host} className="absolute inset-0" />
                <div className="pointer-events-none absolute inset-0">
                    <MarkerLines markers={overlay.markers} />
                    {overlay.budget && (
                        <span
                            className="absolute font-mono text-micro leading-none text-warning"
                            style={{ left: overlay.budget.x, top: overlay.budget.y - 14 }}
                        >
                            {t.charts.tick.budget(format.number(BUDGETS.main))}
                        </span>
                    )}
                    {tooltip && (
                        <span
                            ref={tip}
                            className="absolute top-2 flex -translate-x-1/2 flex-col rounded-sm border border-line-strong bg-surface px-2.5 py-1.5 whitespace-nowrap shadow-pop"
                        >
                            <span className="text-ui font-semibold text-ink">
                                {t.charts.tick.tooltip(
                                    ms(tooltip.main),
                                    ms(tooltip.network),
                                    ms(tooltip.sync),
                                )}
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
