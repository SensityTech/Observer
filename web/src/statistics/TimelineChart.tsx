import {
    AreaSeries,
    CrosshairMode,
    HistogramSeries,
    LineSeries,
    LineStyle,
    createChart,
    type AreaData,
    type AutoscaleInfo,
    type IChartApi,
    type IPriceLine,
    type ISeriesApi,
    type LineData,
    type MouseEventParams,
    type SeriesType,
    type Time,
    type UTCTimestamp,
    type WhitespaceData,
} from 'lightweight-charts';
import {
    Archive,
    FileText,
    GitCompare,
    OctagonX,
    Play,
    RotateCw,
    Square,
    Unplug,
    Zap,
    Boxes,
    type LucideIcon,
} from 'lucide-react';
import {
    useCallback,
    useEffect,
    useLayoutEffect,
    useMemo,
    useRef,
    useState,
    type KeyboardEvent,
    type PointerEvent,
} from 'react';

import type { Timeline } from '../api/generated/Timeline';
import type { TimelineEvent } from '../api/generated/TimelineEvent';
import type { Period } from '../charts/coverage';
import { gapPoints } from '../charts/points';
import { DownBands, RangeBand, instantX, type Extremes } from '../charts/primitives';
import { chartOptions, tokenColor, withAlpha } from '../charts/theme';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { PHONE, useMedia } from '../lib/useMedia';
import { eventLabel, isTrouble, type EventKind } from './events';
import {
    FRAME_BUDGET_MS,
    bucketAt,
    cpuStack,
    frameBand,
    frameLine,
    hitches,
    rowsOf,
    shiftedCpu,
    shiftedRows,
    usualBand,
    type LaneKey,
    type Row,
} from './lanes';

/** Each lane's height, in pixels; a phone gives three quarters. */
const HEIGHTS: Record<LaneKey, number> = {
    players: 150,
    frames: 140,
    cpu: 130,
    memory: 120,
    ping: 100,
    network: 100,
    heap: 100,
    disk: 100,
};
/** The time scale's labels under the last lane. */
const AXIS = 28;
/** A drag shorter than this is a click, not a zoom. */
const LEAST_DRAG = 8;
const SHORTEST_ZOOM = 60;
const LOG_FLOOR = 0.01;

/** Where a frame time sits on the frame lane: its logarithm, from a hundredth of a millisecond. */
const decade = (ms: number) => Math.log10(Math.max(LOG_FLOOR, ms));
const WEEK = 7 * 86_400;

const EVENT_ICONS = {
    start: Play,
    restart: RotateCw,
    stop: Square,
    crash: OctagonX,
    changes: GitCompare,
    resource: Boxes,
    files: FileText,
    crashWave: Zap,
    incident: Unplug,
    backup: Archive,
} satisfies Record<EventKind, LucideIcon>;

type Palette = {
    accent: string;
    ink: string;
    ink2: string;
    ink3: string;
    muted: string;
    warning: string;
    surface: string;
    danger: string;
    dangerSoft: string;
    transparent: string;
};

const palette = (element: Element): Palette => {
    const accent = tokenColor(element, '--accent');
    return {
        accent,
        ink: tokenColor(element, '--ink'),
        ink2: tokenColor(element, '--ink-2'),
        ink3: tokenColor(element, '--ink-3'),
        muted: tokenColor(element, '--chart-muted'),
        warning: tokenColor(element, '--warning-fill'),
        surface: tokenColor(element, '--surface'),
        danger: tokenColor(element, '--danger'),
        dangerSoft: tokenColor(element, '--danger-soft'),
        transparent: withAlpha(accent, 0),
    };
};

/** What one lane draws from. */
export type LaneData = {
    timeline: Timeline;
    compare: Timeline | null;
    forecast: { trend: Row[]; cone: Extremes[] } | null;
    periods: Period[];
};

type Formats = {
    whole: (value: number) => string;
    ms: (value: number) => string;
    percent: (value: number) => string;
    bytes: (value: number) => string;
    bitRate: (value: number) => string;
};

/** A lane once built: the series whose point the crosshair reads, and how it draws and paints. */
type LaneView = {
    key: LaneKey;
    series: ISeriesApi<SeriesType>;
    draw: (data: LaneData) => void;
    paint: (colors: Palette) => void;
};

/**
 * The shared chart options, each lane's scale writing its own unit: the shared price formatter,
 * which counts players, would override them all.
 */
const timelineOptions = (element: HTMLElement, locale: string) => {
    const base = chartOptions(element, locale);
    const { priceFormatter: _, ...localization } = base.localization ?? {};
    return {
        ...base,
        localization,
        layout: {
            ...base.layout,
            panes: {
                enableResize: false,
                separatorColor: tokenColor(element, '--line'),
                separatorHoverColor: 'transparent',
            },
        },
        // A label cut by a lane's edge reads as another number: only whole ones are drawn.
        leftPriceScale: {
            ...base.leftPriceScale,
            minimumWidth: 64,
            entireTextOnly: true,
            scaleMargins: { top: 0.2, bottom: 0.04 },
        },
        crosshair: { ...base.crosshair, mode: CrosshairMode.Normal },
    };
};

const custom = (formatter: (value: number) => string, minMove = 0.01) =>
    ({ type: 'custom', formatter, minMove }) as const;

const quiet = {
    priceScaleId: 'left',
    priceLineVisible: false,
    lastValueVisible: false,
} as const;

const linePoints = (rows: readonly Row[], transparent: string): (LineData | WhitespaceData)[] =>
    gapPoints<LineData>(rows, { color: transparent });

const areaPoints = (rows: readonly Row[], transparent: string): (AreaData | WhitespaceData)[] =>
    gapPoints<AreaData>(rows, { lineColor: transparent, topColor: transparent, bottomColor: transparent });

/** One stacked area's line and fill. */
const shade = (series: ISeriesApi<'Area'>, color: string, alpha: number) =>
    series.applyOptions({
        lineColor: color,
        topColor: withAlpha(color, alpha),
        bottomColor: withAlpha(color, alpha),
        crosshairMarkerVisible: false,
    });

/** The highest value of rows and bands, for a scale that keeps a band in view. */
const highest = (values: readonly (number | null | undefined)[]) =>
    values.reduce<number>((top, value) => (value == null ? top : Math.max(top, value)), 0);

/** A scale from 0 to a little above the highest of what the lane draws. */
const fromZero =
    (top: () => number) =>
    (original: () => AutoscaleInfo | null): AutoscaleInfo | null => {
        const range = original()?.priceRange ?? null;
        const max = Math.max(range?.maxValue ?? 0, top());
        return { priceRange: { minValue: 0, maxValue: max > 0 ? max * 1.08 : 1 } };
    };

function buildLanes(
    api: IChartApi,
    lanes: readonly LaneKey[],
    comparing: boolean,
    format: Formats,
): LaneView[] {
    let colors: Palette | null = null;
    const transparent = () => colors?.transparent ?? 'transparent';
    return lanes.map((key, pane): LaneView => {
        const line = (options: { width?: 1 | 2; dashed?: boolean; formatter: (value: number) => string }) =>
            api.addSeries(
                LineSeries,
                {
                    ...quiet,
                    lineWidth: options.width ?? 1,
                    lineStyle: options.dashed ? LineStyle.Dashed : LineStyle.Solid,
                    crosshairMarkerVisible: !options.dashed,
                    priceFormat: custom(options.formatter),
                },
                pane,
            );
        const area = (formatter: (value: number) => string, width: 1 | 2 = 2) =>
            api.addSeries(
                AreaSeries,
                { ...quiet, lineWidth: width, crosshairMarkerRadius: 4, priceFormat: custom(formatter) },
                pane,
            );
        let last: LaneData | null = null;

        switch (key) {
            case 'players': {
                let top = 0;
                const online = area(format.whole);
                online.applyOptions({ autoscaleInfoProvider: fromZero(() => top) });
                const usual = new RangeBand();
                const down = new DownBands();
                online.attachPrimitive(usual);
                online.attachPrimitive(down);
                const week = comparing ? line({ dashed: true, formatter: format.whole }) : null;
                const bands = () => {
                    if (last === null || colors === null) {
                        return;
                    }
                    usual.set(usualBand(last.timeline), withAlpha(colors.ink, 0.14));
                    down.set(last.periods, last.timeline.step, {
                        fill: colors.dangerSoft,
                        edge: colors.danger,
                    });
                };
                return {
                    key,
                    series: online,
                    draw: (data) => {
                        last = data;
                        top = highest(data.timeline.usual?.high ?? []);
                        online.setData(areaPoints(rowsOf(data.timeline, 'players'), transparent()));
                        week?.setData(
                            data.compare
                                ? linePoints(shiftedRows(data.compare, 'players'), transparent())
                                : [],
                        );
                        bands();
                    },
                    paint: (scheme) => {
                        colors = scheme;
                        online.applyOptions({
                            lineColor: scheme.accent,
                            topColor: withAlpha(scheme.accent, 0.1),
                            bottomColor: withAlpha(scheme.accent, 0.1),
                            crosshairMarkerBorderColor: scheme.surface,
                            crosshairMarkerBackgroundColor: scheme.accent,
                        });
                        week?.applyOptions({ color: scheme.ink3 });
                        bands();
                    },
                };
            }
            case 'frames': {
                // Milliseconds spread over decades: the lane draws their logarithm on its scale,
                // which the lanes share with every other, and writes them back as milliseconds.
                let range = { low: 1, high: FRAME_BUDGET_MS };
                const ms = (value: number) => format.ms(10 ** value);
                const median = line({ width: 2, formatter: ms });
                median.applyOptions({
                    autoscaleInfoProvider: () => ({
                        priceRange: {
                            minValue: decade(range.low / 2),
                            maxValue: decade(Math.max(FRAME_BUDGET_MS * 1.2, range.high * 1.3)),
                        },
                    }),
                });
                const tail = new RangeBand();
                median.attachPrimitive(tail);
                const budget: IPriceLine = median.createPriceLine({
                    price: decade(FRAME_BUDGET_MS),
                    lineWidth: 1,
                    lineStyle: LineStyle.Dashed,
                    axisLabelVisible: false,
                    title: '',
                });
                const marks = api.addSeries(
                    HistogramSeries,
                    {
                        priceScaleId: 'hitches',
                        priceLineVisible: false,
                        lastValueVisible: false,
                        priceFormat: custom(format.whole, 1),
                    },
                    pane,
                );
                api.priceScale('hitches', pane).applyOptions({
                    visible: false,
                    scaleMargins: { top: 0.88, bottom: 0 },
                });
                const week = comparing ? line({ dashed: true, formatter: ms }) : null;
                const clamp = (rows: Row[]) =>
                    rows.map((row) => ({ ...row, value: row.value === null ? null : decade(row.value) }));
                const band = () => {
                    if (last !== null && colors !== null) {
                        tail.set(
                            frameBand(last.timeline).map((point) => ({
                                ...point,
                                min: point.min === null ? null : decade(point.min),
                                max: point.max === null ? null : decade(point.max),
                            })),
                            withAlpha(colors.accent, 0.28),
                        );
                    }
                };
                return {
                    key,
                    series: median,
                    draw: (data) => {
                        last = data;
                        const shown = frameLine(data.timeline).rows;
                        const lows = shown.map((row) => row.value).filter((value) => value !== null);
                        range = {
                            low: lows.length > 0 ? Math.min(...lows) : 1,
                            high: highest([...(data.timeline.frames?.p99 ?? []), ...lows]),
                        };
                        median.setData(linePoints(clamp(shown), transparent()));
                        const marked = new Set(hitches(data.timeline).map((hitch) => hitch.time));
                        marks.setData(
                            rowsOf(data.timeline, 'hitchesMain').map((row) =>
                                marked.has(row.time)
                                    ? { time: row.time as UTCTimestamp, value: 1 }
                                    : { time: row.time as UTCTimestamp },
                            ),
                        );
                        week?.setData(
                            data.compare
                                ? linePoints(
                                      clamp(
                                          frameLine(data.compare).rows.map((row) => ({
                                              ...row,
                                              time: row.time + WEEK,
                                          })),
                                      ),
                                      transparent(),
                                  )
                                : [],
                        );
                        band();
                    },
                    paint: (scheme) => {
                        colors = scheme;
                        median.applyOptions({
                            color: scheme.accent,
                            crosshairMarkerBorderColor: scheme.surface,
                            crosshairMarkerBackgroundColor: scheme.accent,
                        });
                        budget.applyOptions({ color: scheme.warning });
                        marks.applyOptions({ color: scheme.warning });
                        week?.applyOptions({ color: scheme.ink3 });
                        band();
                    },
                };
            }
            case 'cpu': {
                if (comparing) {
                    const total = area(format.percent);
                    total.applyOptions({ autoscaleInfoProvider: fromZero(() => 0) });
                    const week = line({ dashed: true, formatter: format.percent });
                    return {
                        key,
                        series: total,
                        draw: (data) => {
                            last = data;
                            total.setData(areaPoints(cpuStack(data.timeline).all, transparent()));
                            week.setData(
                                data.compare ? linePoints(shiftedCpu(data.compare), transparent()) : [],
                            );
                        },
                        paint: (scheme) => {
                            colors = scheme;
                            total.applyOptions({
                                lineColor: scheme.accent,
                                topColor: withAlpha(scheme.accent, 0.1),
                                bottomColor: withAlpha(scheme.accent, 0.1),
                            });
                            week.applyOptions({ color: scheme.ink3 });
                        },
                    };
                }
                const all = area(format.percent, 1);
                const server = area(format.percent, 1);
                const main = area(format.percent, 1);
                for (const series of [all, server, main]) {
                    series.applyOptions({ autoscaleInfoProvider: fromZero(() => 0) });
                }
                return {
                    key,
                    series: all,
                    draw: (data) => {
                        last = data;
                        const stack = cpuStack(data.timeline);
                        all.setData(areaPoints(stack.all, transparent()));
                        server.setData(areaPoints(stack.server, transparent()));
                        main.setData(areaPoints(stack.main, transparent()));
                    },
                    paint: (scheme) => {
                        colors = scheme;
                        shade(all, scheme.muted, 0.16);
                        shade(server, scheme.ink2, 0.3);
                        shade(main, scheme.accent, 0.5);
                    },
                };
            }
            case 'memory': {
                let extent = { low: Number.POSITIVE_INFINITY, high: 0 };
                const used = area(format.bytes);
                used.applyOptions({
                    autoscaleInfoProvider: (original: () => AutoscaleInfo | null) => {
                        const range = original()?.priceRange ?? null;
                        if (range === null) {
                            return null;
                        }
                        const low = Math.min(range.minValue, extent.low);
                        const high = Math.max(range.maxValue, extent.high);
                        const margin = (high - low) * 0.08 || high * 0.05;
                        return {
                            priceRange: { minValue: Math.max(0, low - margin), maxValue: high + margin },
                        };
                    },
                });
                const cone = new RangeBand();
                used.attachPrimitive(cone);
                const trend = line({ dashed: true, formatter: format.bytes });
                const week = comparing ? line({ dashed: true, formatter: format.bytes }) : null;
                const band = () => {
                    if (last !== null && colors !== null) {
                        cone.set(last.forecast?.cone ?? [], withAlpha(colors.accent, 0.16));
                    }
                };
                return {
                    key,
                    series: used,
                    draw: (data) => {
                        last = data;
                        const cones = data.forecast?.cone ?? [];
                        extent = {
                            low: Math.min(...cones.map((point) => point.min ?? Number.POSITIVE_INFINITY)),
                            high: highest(cones.map((point) => point.max)),
                        };
                        used.setData(areaPoints(rowsOf(data.timeline, 'fxsPrivate'), transparent()));
                        trend.setData(data.forecast ? linePoints(data.forecast.trend, transparent()) : []);
                        week?.setData(
                            data.compare
                                ? linePoints(shiftedRows(data.compare, 'fxsPrivate'), transparent())
                                : [],
                        );
                        band();
                    },
                    paint: (scheme) => {
                        colors = scheme;
                        used.applyOptions({
                            lineColor: scheme.accent,
                            topColor: withAlpha(scheme.accent, 0.1),
                            bottomColor: withAlpha(scheme.accent, 0.1),
                            crosshairMarkerBorderColor: scheme.surface,
                            crosshairMarkerBackgroundColor: scheme.accent,
                        });
                        trend.applyOptions({ color: scheme.ink2 });
                        week?.applyOptions({ color: scheme.ink3 });
                        band();
                    },
                };
            }
            default: {
                const [first, second, formatter] = (
                    {
                        ping: ['pingAvg', 'pingP95', format.ms],
                        network: ['hostReceive', 'hostSend', format.bitRate],
                        heap: ['jsHeapUsed', 'jsHeapTotal', format.bytes],
                        disk: ['hostDiskLatency', null, format.ms],
                    } as const
                )[key];
                const main = line({ width: 2, formatter });
                main.applyOptions({ autoscaleInfoProvider: fromZero(() => 0) });
                const other = second === null ? null : line({ dashed: key === 'heap', formatter });
                other?.applyOptions({ autoscaleInfoProvider: fromZero(() => 0) });
                return {
                    key,
                    series: main,
                    draw: (data) => {
                        last = data;
                        main.setData(linePoints(rowsOf(data.timeline, first), transparent()));
                        if (other !== null && second !== null) {
                            other.setData(linePoints(rowsOf(data.timeline, second), transparent()));
                        }
                    },
                    paint: (scheme) => {
                        colors = scheme;
                        main.applyOptions({
                            color: scheme.accent,
                            crosshairMarkerBorderColor: scheme.surface,
                            crosshairMarkerBackgroundColor: scheme.accent,
                        });
                        other?.applyOptions({ color: key === 'heap' ? scheme.ink3 : scheme.ink2 });
                    },
                };
            }
        }
    });
}

type Placed = {
    /** The plot's left edge inside the host: the price scale's width. */
    left: number;
    panes: { key: LaneKey; top: number }[];
    events: { event: TimelineEvent; x: number; until: number | null }[];
    now: number | null;
    cursor: number | null;
};

export type TimelineChartProps = {
    timeline: Timeline;
    compare: Timeline | null;
    lanes: readonly LaneKey[];
    events: readonly TimelineEvent[];
    forecast: LaneData['forecast'];
    periods: Period[];
    /** Unix seconds, when the window ends now: the forecast starts there. */
    now: number | null;
    cursor: number | null;
    onCursor: (bucket: number | null) => void;
    onZoom: (from: number, to: number) => void;
    onReset: () => void;
    onEvent: (event: TimelineEvent) => void;
    labelledBy: string;
    describedBy: string;
    /** Where the cursor is, in words, for the slider it is to assistive technology. */
    valueText: string;
    legends: Record<LaneKey, { title: string; keys: { label: string; swatch: string }[] }>;
};

export default function TimelineChart(props: TimelineChartProps) {
    const {
        timeline,
        compare,
        lanes,
        events,
        forecast,
        periods,
        now,
        cursor,
        onCursor,
        onZoom,
        onReset,
        onEvent,
    } = props;
    const { t, format, locale } = useI18n();
    const phone = useMedia(PHONE);
    const host = useRef<HTMLDivElement>(null);
    const chart = useRef<IChartApi | null>(null);
    const views = useRef<LaneView[]>([]);
    const hovering = useRef(false);
    const [placed, setPlaced] = useState<Placed>({ left: 0, panes: [], events: [], now: null, cursor: null });
    const [drag, setDrag] = useState<{ from: number; to: number } | null>(null);
    const shape = `${lanes.join(',')}:${compare !== null}:${phone}:${locale}`;
    const heights = useMemo(
        () => lanes.map((lane) => Math.round(HEIGHTS[lane] * (phone ? 0.75 : 1))),
        [lanes, phone],
    );
    const height = heights.reduce((sum, each) => sum + each, 0) + AXIS;
    const formats = useMemo<Formats>(
        () => ({
            whole: (value) => format.number(Math.round(value)),
            ms: (value) => format.milliseconds(value),
            percent: (value) => format.percent(value),
            bytes: (value) => format.bytes(value),
            bitRate: (value) => format.bitRate(value),
        }),
        [format],
    );

    // What the chart is built and filled from, read by its handlers without building it again.
    const current = useRef({ lanes, comparing: compare !== null, formats, heights, locale, onCursor });
    const data = useRef<LaneData>({ timeline, compare, forecast, periods });
    useLayoutEffect(() => {
        current.current = { lanes, comparing: compare !== null, formats, heights, locale, onCursor };
        data.current = { timeline, compare, forecast, periods };
    });

    const placing = useRef(() => {});
    const place = useCallback(() => {
        const api = chart.current;
        if (api === null) {
            return;
        }
        const scale = api.timeScale();
        const left = api.priceScale('left', 0).width();
        const starts = Array.from(
            { length: timeline.buckets },
            (_, index) => timeline.from + index * timeline.step,
        );
        const x = (time: number) => instantX(scale, starts, timeline.step, time);
        let top = 0;
        const panes = api.panes().map((pane, index) => {
            const at = top;
            top += pane.getHeight() + 1;
            return { key: lanes[index] ?? 'players', top: at };
        });
        setPlaced({
            left,
            panes,
            events: events.flatMap((event) => {
                const at = x(event.at);
                if (at === null) {
                    return [];
                }
                const until = event.until === null ? null : x(event.until);
                return [{ event, x: at, until }];
            }),
            now: now === null ? null : x(now),
            cursor: cursor === null ? null : x(timeline.from + cursor * timeline.step),
        });
    }, [timeline, events, lanes, now, cursor]);

    // Built again only when the lanes, the comparison or the layout change: `shape` says so.
    useEffect(() => {
        const element = host.current;
        if (element === null || shape === '') {
            return;
        }
        const {
            lanes: keys,
            comparing,
            formats: formatters,
            heights: sizes,
            locale: language,
        } = current.current;
        const api = createChart(element, timelineOptions(element, language));
        chart.current = api;
        const built = buildLanes(api, keys, comparing, formatters);
        views.current = built;
        // Each lane keeps its share of the height, whatever the plot's own.
        api.panes().forEach((pane, index) => pane.setStretchFactor(sizes[index] ?? 80));
        const paint = () => {
            const colors = palette(element);
            api.applyOptions(timelineOptions(element, language));
            for (const view of built) {
                view.paint(colors);
            }
        };
        paint();
        for (const view of built) {
            view.draw(data.current);
        }
        api.timeScale().fitContent();
        const placeNow = () => placing.current();
        const scale = api.timeScale();
        scale.subscribeVisibleLogicalRangeChange(placeNow);
        const resized = new ResizeObserver(placeNow);
        resized.observe(element);
        const theme = new MutationObserver(paint);
        theme.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
        const moved = (event: MouseEventParams<Time>) => {
            if (event.time === undefined || event.point === undefined) {
                if (hovering.current) {
                    current.current.onCursor(null);
                }
                return;
            }
            hovering.current = true;
            current.current.onCursor(bucketAt(data.current.timeline, event.time as number));
        };
        api.subscribeCrosshairMove(moved);
        return () => {
            theme.disconnect();
            resized.disconnect();
            scale.unsubscribeVisibleLogicalRangeChange(placeNow);
            api.unsubscribeCrosshairMove(moved);
            api.remove();
            chart.current = null;
            views.current = [];
        };
    }, [shape]);

    useEffect(() => {
        for (const view of views.current) {
            view.draw({ timeline, compare, forecast, periods });
        }
        chart.current?.timeScale().fitContent();
    }, [timeline, compare, forecast, periods]);

    useEffect(() => {
        placing.current = place;
        place();
    }, [place]);

    // The keyboard moves the crosshair as the pointer does.
    useEffect(() => {
        const api = chart.current;
        const first = views.current[0];
        if (api === null || first === undefined || hovering.current) {
            return;
        }
        if (cursor === null) {
            api.clearCrosshairPosition();
            return;
        }
        const time = (timeline.from + cursor * timeline.step) as UTCTimestamp;
        const value = rowsOf(timeline, 'players')[cursor]?.value ?? 0;
        api.setCrosshairPosition(value, time, first.series);
    }, [cursor, timeline]);

    const timeAt = (offsetX: number) => {
        const api = chart.current;
        if (api === null) {
            return null;
        }
        const time = api.timeScale().coordinateToTime(offsetX - placed.left);
        return time === null ? null : (time as number);
    };

    const pointerDown = (event: PointerEvent<HTMLDivElement>) => {
        if (event.button !== 0 || event.pointerType === 'touch') {
            return;
        }
        const x = event.nativeEvent.offsetX;
        setDrag({ from: x, to: x });
    };
    const pointerMove = (event: PointerEvent<HTMLDivElement>) => {
        if (drag !== null) {
            const x = event.nativeEvent.offsetX;
            setDrag({ ...drag, to: x });
        }
    };
    const pointerUp = () => {
        if (drag === null) {
            return;
        }
        setDrag(null);
        if (Math.abs(drag.to - drag.from) < LEAST_DRAG) {
            return;
        }
        const a = timeAt(Math.min(drag.from, drag.to));
        const b = timeAt(Math.max(drag.from, drag.to));
        if (a !== null && b !== null) {
            const to = b + timeline.step;
            onZoom(a, Math.max(to, a + SHORTEST_ZOOM));
        }
    };

    // The range moves the cursor by itself; Shift moves it ten buckets, + and − zoom about it.
    const keyDown = (event: KeyboardEvent<HTMLInputElement>) => {
        hovering.current = false;
        const last = timeline.buckets - 1;
        const at = cursor ?? last;
        const moveTo = (index: number) => {
            event.preventDefault();
            onCursor(Math.max(0, Math.min(last, index)));
        };
        switch (event.key) {
            case 'ArrowLeft':
            case 'ArrowDown':
                return event.shiftKey ? moveTo(at - 10) : undefined;
            case 'ArrowRight':
            case 'ArrowUp':
                return event.shiftKey ? moveTo(at + 10) : undefined;
            case '+':
            case '=': {
                event.preventDefault();
                const span = (timeline.buckets * timeline.step) / 4;
                const middle = timeline.from + at * timeline.step;
                onZoom(Math.round(middle - span), Math.round(middle + span));
                return;
            }
            case '-': {
                event.preventDefault();
                const span = timeline.buckets * timeline.step;
                const middle = timeline.from + at * timeline.step;
                onZoom(Math.round(middle - span), Math.round(middle + span));
                return;
            }
            case 'Escape':
                event.preventDefault();
                onReset();
                return;
        }
    };

    const words = {
        duration: (seconds: number) => format.duration(seconds),
        clock: (seconds: number) => format.dateTime(new Date(seconds * 1000), { dateStyle: undefined }),
        ms: (ms: number) => format.milliseconds(ms),
    };

    return (
        <div className="flex flex-col">
            <EventStrip
                placed={placed}
                onEvent={onEvent}
                label={(event) => eventLabel(t, event, words)}
                narrow={phone}
            />
            {/* The cursor is a range over the buckets for the keyboard and assistive technology;
                the plot beside it is drawn for the eye and the pointer. */}
            <input
                type="range"
                className="peer sr-only"
                min={0}
                max={timeline.buckets - 1}
                value={cursor ?? timeline.buckets - 1}
                aria-labelledby={props.labelledBy}
                aria-describedby={props.describedBy}
                aria-valuetext={props.valueText}
                onChange={(event) => {
                    hovering.current = false;
                    onCursor(Number(event.target.value));
                }}
                onKeyDown={keyDown}
            />
            <div
                aria-hidden="true"
                onPointerDown={pointerDown}
                onPointerMove={pointerMove}
                onPointerUp={pointerUp}
                onPointerLeave={() => {
                    setDrag(null);
                    hovering.current = false;
                    onCursor(null);
                }}
                className="relative touch-pan-y rounded-sm outline-offset-4 select-none peer-focus-visible:outline-2 peer-focus-visible:outline-focus-ring"
                style={{ height }}
            >
                <div ref={host} className="absolute inset-0" />
                <div className="pointer-events-none absolute inset-0" aria-hidden="true">
                    {placed.now !== null && forecast !== null && (
                        <div
                            className="absolute top-0 right-0 border-l border-dashed border-ink-3 bg-bg-subtle/60"
                            style={{ left: placed.left + placed.now, bottom: AXIS }}
                        >
                            <span className="absolute top-1 left-2 font-mono text-micro tracking-label text-ink-3 uppercase">
                                {t.statistics.timeline.forecast}
                            </span>
                        </div>
                    )}
                    {placed.panes.map((pane) => (
                        <LaneLegend
                            key={pane.key}
                            top={pane.top}
                            left={placed.left}
                            legend={props.legends[pane.key]}
                        />
                    ))}
                    {drag !== null && (
                        <div
                            className="absolute top-0 border-x border-accent bg-accent/10"
                            style={{
                                left: Math.min(drag.from, drag.to),
                                width: Math.abs(drag.to - drag.from),
                                bottom: AXIS,
                            }}
                        />
                    )}
                </div>
            </div>
        </div>
    );
}

function LaneLegend({
    top,
    left,
    legend,
}: {
    top: number;
    left: number;
    legend: { title: string; keys: { label: string; swatch: string }[] };
}) {
    return (
        <div
            className="absolute flex flex-wrap items-center gap-x-3 gap-y-0.5 pr-2 font-mono text-micro tracking-label text-ink-3 uppercase"
            style={{ top: top + 4, left: left + 8 }}
        >
            <span className="text-ink-2">{legend.title}</span>
            {legend.keys.map((key) => (
                <span key={key.label} className="flex items-center gap-1.5 max-md:hidden">
                    <span className={key.swatch} />
                    {key.label}
                </span>
            ))}
        </div>
    );
}

/** Room a label takes right of its instant. */
const LABEL_ROOM = 150;

/**
 * Which events, oldest first at `xs`, have room for their label right of their instant: the next
 * event is far enough and no label before it reaches it.
 */
export const labelSpots = (xs: readonly number[]) => {
    let free = Number.NEGATIVE_INFINITY;
    return xs.map((x, index) => {
        const next = xs[index + 1] ?? Number.POSITIVE_INFINITY;
        const fits = x >= free && next - x > LABEL_ROOM;
        if (fits) {
            free = x + LABEL_ROOM;
        }
        return fits;
    });
};

/** The band above the plot: each event's icon at its instant, its label beside it when it fits. */
function EventStrip({
    placed,
    onEvent,
    label,
    narrow,
}: {
    placed: Placed;
    onEvent: (event: TimelineEvent) => void;
    label: (event: TimelineEvent) => string;
    narrow: boolean;
}) {
    const labelled = narrow ? [] : labelSpots(placed.events.map(({ x }) => x));
    return (
        <div className="relative h-7 border-b border-line" aria-hidden="true">
            {placed.events.map(({ event, x, until }, index) => {
                const Icon = EVENT_ICONS[event.detail.kind];
                const left = placed.left + x;
                const trouble = isTrouble(event);
                return (
                    <span key={event.key}>
                        {until !== null && (
                            <span
                                className={cn(
                                    'absolute top-1.5 h-4 rounded-[3px] border',
                                    trouble
                                        ? 'border-danger bg-danger-soft'
                                        : 'border-line-strong bg-surface-2',
                                )}
                                style={{ left, width: Math.max(6, until - x) }}
                            />
                        )}
                        <button
                            type="button"
                            tabIndex={-1}
                            onClick={() => onEvent(event)}
                            className={cn(
                                'absolute top-1 flex h-5 -translate-x-1/2 items-center gap-1 rounded-pill bg-surface px-1 text-caption whitespace-nowrap',
                                trouble ? 'text-danger-ink' : 'text-ink-2',
                                labelled[index] && 'translate-x-[-10px]',
                            )}
                            style={{ left }}
                        >
                            <Icon size={12} strokeWidth={2} />
                            {labelled[index] && <span>{label(event)}</span>}
                        </button>
                    </span>
                );
            })}
        </div>
    );
}
