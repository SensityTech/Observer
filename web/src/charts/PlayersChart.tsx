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
import { OctagonX, Play, RotateCw, Users, type LucideIcon } from 'lucide-react';
import { useCallback, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from 'react';

import type { MarkerKind } from '../api/generated/MarkerKind';
import type { MetricsSeries } from '../api/generated/MetricsSeries';
import type { Range } from '../api/generated/Range';
import type { RunMarker } from '../api/generated/RunMarker';
import { seriesQuery } from '../api/metrics';
import { useI18n } from '../i18n/I18nProvider';
import { formatTimestamp } from '../i18n/format';
import { cn } from '../lib/cn';
import { useLiveMetrics } from '../metrics/live';
import { mergeLive } from '../metrics/merge';
import { bucketOf, sides, type Side } from './markers';
import { buckets, playerPoints, resumes, slotPoints, type Bucket } from './points';
import { scaleTop } from './scale';
import { chartOptions, tokenColor, withAlpha } from './theme';

const RANGES: Range[] = ['1h', '6h', '24h', '7d', '30d'];
const SERIES = ['players', 'slots'] as const;
const ATTRIBUTION = 'https://www.tradingview.com/';

const MARKER_ICONS = { start: Play, restart: RotateCw, crash: OctagonX } satisfies Record<
    MarkerKind,
    LucideIcon
>;

const pad = (value: number) => String(value).padStart(2, '0');
const clock = (seconds: number) => {
    const date = new Date(seconds * 1000);
    return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
};

type Overlay = {
    markers: { marker: RunMarker; x: number }[];
    peak: { x: number; y: number } | null;
    slots: { y: number; value: number } | null;
};

export default function PlayersChart() {
    const { t, format } = useI18n();
    const [range, setRange] = useState<Range>('24h');
    const query = useQuery({ ...seriesQuery(range, SERIES), placeholderData: keepPreviousData });
    const { recent, latest } = useLiveMetrics();
    const page = useMemo(() => (query.data ? mergeLive(query.data, recent) : null), [query.data, recent]);
    const rows = useMemo(() => (page ? buckets(page) : []), [page]);
    const measured = rows.filter((row) => row.players !== null);
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
        const at =
            page.step >= 900
                ? formatTimestamp(new Date(peak.time * 1000), { seconds: false })
                : clock(peak.time);
        const figures = [
            format.number(Math.round(average)),
            range,
            format.number(Math.round(peak.players)),
            at,
        ] as const;
        summary =
            newest === null
                ? t.charts.players.summary(...figures)
                : t.charts.players.summaryNow(format.number(newest), ...figures);
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
                    {t.charts.players.title}
                </h2>
                {page && (
                    <span className="text-meta text-ink-3">
                        {t.charts.players.every(format.duration(page.step))}
                    </span>
                )}
                <fieldset className="ml-auto inline-flex max-w-full gap-0.5 rounded-control border border-line bg-surface-2 p-0.75 md:-my-1.5">
                    <legend className="sr-only">{t.charts.players.range}</legend>
                    {RANGES.map((choice) => (
                        <button
                            key={choice}
                            type="button"
                            aria-pressed={range === choice}
                            onClick={() => setRange(choice)}
                            className="inline-flex h-6.5 items-center rounded-[7px] px-2.5 text-small font-medium text-ink-3 transition-colors hover:text-ink focus-visible:outline-offset-1 aria-pressed:bg-raise aria-pressed:text-ink aria-pressed:shadow-[var(--elev-sm),0_0_0_1px_var(--line)] max-md:h-11 max-md:min-w-11 max-md:justify-center"
                        >
                            {choice}
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
                    {t.charts.players.players}
                </span>
                <span className="flex items-center gap-1.5">
                    <span className="w-3.5 border-t-[1.5px] border-dashed border-ink-3" />
                    {t.charts.players.maxSlots}
                </span>
                <span className="flex items-center gap-1.5">
                    <span className="h-3 border-l border-dashed border-ink-3" />
                    {t.charts.players.event}
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
                            top={top}
                            peakTime={peak?.time ?? null}
                        />
                        {summary !== null && (
                            <figcaption className="mt-4 border-t border-line pt-3 text-ui text-ink-3">
                                {summary}
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
                    TradingView Lightweight Charts™
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
    top,
    peakTime,
}: {
    page: MetricsSeries;
    /** The page as read, before live samples: a new one is drawn anew. */
    fetched: MetricsSeries | null;
    rows: Bucket[];
    top: number;
    peakTime: number | null;
}) {
    const { t, format } = useI18n();
    const host = useRef<HTMLDivElement>(null);
    const chart = useRef<IChartApi | null>(null);
    const players = useRef<ISeriesApi<'Area'> | null>(null);
    const slots = useRef<ISeriesApi<'Line'> | null>(null);
    const drawn = useRef<MetricsSeries | null>(null);
    const topRef = useRef(top);
    const [overlay, setOverlay] = useState<Overlay>({ markers: [], peak: null, slots: null });
    const [tooltip, setTooltip] = useState<{ x: number; players: number; time: number } | null>(null);
    const [layout, setLayout] = useState<Side[]>([]);
    const labels = useRef<(HTMLSpanElement | null)[]>([]);
    const band = useRef<HTMLDivElement>(null);

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
            slots: latest === null || slotsY === null ? null : { y: slotsY, value: latest },
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
        const api = createChart(element, chartOptions(element));
        const autoscale = () => ({ priceRange: { minValue: 0, maxValue: topRef.current } });
        players.current = api.addSeries(AreaSeries, {
            priceScaleId: 'left',
            lineWidth: 2,
            priceLineVisible: false,
            lastValueVisible: false,
            crosshairMarkerRadius: 4,
            autoscaleInfoProvider: autoscale,
        });
        slots.current = api.addSeries(LineSeries, {
            priceScaleId: 'left',
            lineWidth: 1,
            lineStyle: LineStyle.Dashed,
            lineType: LineType.WithSteps,
            priceLineVisible: false,
            lastValueVisible: false,
            crosshairMarkerVisible: false,
            autoscaleInfoProvider: autoscale,
        });
        chart.current = api;
        const paint = () => {
            const accent = tokenColor(element, '--accent');
            api.applyOptions(chartOptions(element));
            players.current?.applyOptions({
                lineColor: accent,
                topColor: withAlpha(accent, 0.1),
                bottomColor: withAlpha(accent, 0.1),
                crosshairMarkerBorderColor: tokenColor(element, '--surface'),
                crosshairMarkerBackgroundColor: accent,
            });
            slots.current?.applyOptions({ color: tokenColor(element, '--ink-3') });
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
    }, []);

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

    useLayoutEffect(() => {
        const next = sides(
            overlay.markers.map(({ marker, x }, index) => ({
                x,
                width: labels.current[index]?.offsetWidth ?? 0,
                weight: marker.kind === 'crash' ? 1 : 0,
            })),
            band.current?.offsetWidth ?? 0,
        );
        setLayout((current) => (current.join() === next.join() ? current : next));
    }, [overlay]);

    const label = (marker: RunMarker) => {
        const word = t.charts.markers[marker.kind];
        const time = clock(marker.at);
        const back =
            marker.kind === 'crash' && marker.backOnlineAt !== null
                ? ` · ${t.charts.markers.backOnline(clock(marker.backOnlineAt))}`
                : '';
        return [word, time, back] as const;
    };

    return (
        <div className="relative flex flex-1 flex-col" aria-hidden="true">
            <div ref={band} className="relative h-6 overflow-hidden" aria-hidden="true">
                {overlay.markers.map(({ marker, x }, index) => {
                    const Icon = MARKER_ICONS[marker.kind];
                    const right = layout[index] === 'right';
                    const [word, time, back] = label(marker);
                    return (
                        <span
                            key={`${marker.kind}-${marker.at}`}
                            ref={(element) => {
                                labels.current[index] = element;
                            }}
                            className={cn(
                                'absolute top-0 flex h-5 items-center gap-1 text-caption leading-none whitespace-nowrap text-ink-2',
                                right ? 'pl-1.75' : 'pr-1.75',
                                layout[index] === 'hidden' && 'invisible',
                            )}
                            style={{ left: x, transform: right ? undefined : 'translateX(-100%)' }}
                        >
                            {right && <span className="font-mono text-[11px] text-ink-3">{time}</span>}
                            <Icon
                                size={12}
                                strokeWidth={2}
                                className={marker.kind === 'crash' ? 'text-danger' : 'text-ink-3'}
                            />
                            {word}
                            {!right && <span className="font-mono text-[11px] text-ink-3">{time}</span>}
                            {back}
                        </span>
                    );
                })}
            </div>
            <div className="relative h-[260px] max-md:h-[200px]">
                <div ref={host} className="absolute inset-0" />
                <div className="pointer-events-none absolute inset-0" aria-hidden="true">
                    {overlay.markers.map(({ marker, x }) => (
                        <span
                            key={`${marker.kind}-${marker.at}`}
                            className={cn(
                                'absolute -top-6 bottom-0 border-l border-dashed',
                                marker.kind === 'crash' ? 'border-danger' : 'border-ink-3',
                            )}
                            style={{ left: x }}
                        />
                    ))}
                    {overlay.slots && (
                        <span
                            className="absolute left-12 font-mono text-micro leading-none text-ink-3"
                            style={{ top: overlay.slots.y - 14 }}
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
                            className="absolute top-2 flex -translate-x-1/2 flex-col rounded-sm border border-line-strong bg-surface px-2.5 py-1.5 whitespace-nowrap shadow-pop"
                            style={{ left: tooltip.x }}
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
