import {
    AreaSeries,
    HistogramSeries,
    LineSeries,
    createChart,
    type IChartApi,
    type ISeriesApi,
    type MouseEventParams,
    type Time,
} from 'lightweight-charts';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';

import { dayBefore, type CommunityPoint } from '../api/players';
import { useI18n } from '../i18n/I18nProvider';
import { dayChartOptions } from './days';
import { ceiling } from './scale';
import { tokenColor, withAlpha } from './theme';

/** Players are whole: the scale steps and reads in whole numbers. */
const WHOLE = { type: 'price', precision: 0, minMove: 1 } as const;
/** Play time's own scale, laid over the lower part of the plot, without an axis. */
const PLAY = 'play';

/**
 * Unique players as the main area, new players as a line on the same scale, and play time as bars
 * of their own scale. Built once; the points and the colours change in place.
 */
export function CommunityChart({ points, weekly }: { points: CommunityPoint[]; weekly: boolean }) {
    const { t, format, locale } = useI18n();
    const host = useRef<HTMLDivElement>(null);
    const chart = useRef<IChartApi | null>(null);
    const players = useRef<ISeriesApi<'Area'> | null>(null);
    const newcomers = useRef<ISeriesApi<'Line'> | null>(null);
    const played = useRef<ISeriesApi<'Histogram'> | null>(null);
    const localeRef = useRef(locale);
    const shown = useRef(points);
    const top = ceiling(
        Math.max(0, ...points.map((point) => Math.max(point.players ?? 0, point.newPlayers))),
        null,
    );
    const topRef = useRef(top);
    const [hovered, setHovered] = useState<{ x: number; point: CommunityPoint } | null>(null);
    const tip = useRef<HTMLSpanElement>(null);

    useEffect(() => {
        topRef.current = top;
        shown.current = points;
    }, [top, points]);

    useEffect(() => {
        const element = host.current;
        if (element === null) {
            return;
        }
        const api = createChart(element, dayChartOptions(element, localeRef.current));
        const autoscale = () => ({ priceRange: { minValue: 0, maxValue: topRef.current } });
        // Drawn first, so that the players' area and line stay over the bars.
        played.current = api.addSeries(HistogramSeries, {
            priceScaleId: PLAY,
            priceLineVisible: false,
            lastValueVisible: false,
        });
        players.current = api.addSeries(AreaSeries, {
            priceScaleId: 'left',
            priceFormat: WHOLE,
            lineWidth: 2,
            priceLineVisible: false,
            lastValueVisible: false,
            crosshairMarkerRadius: 4,
            autoscaleInfoProvider: autoscale,
        });
        newcomers.current = api.addSeries(LineSeries, {
            priceScaleId: 'left',
            priceFormat: WHOLE,
            lineWidth: 1,
            priceLineVisible: false,
            lastValueVisible: false,
            crosshairMarkerVisible: false,
            autoscaleInfoProvider: autoscale,
        });
        api.priceScale(PLAY).applyOptions({ scaleMargins: { top: 0.75, bottom: 0 } });
        chart.current = api;
        const paint = () => {
            const accent = tokenColor(element, '--accent');
            api.applyOptions(dayChartOptions(element, localeRef.current));
            players.current?.applyOptions({
                lineColor: accent,
                topColor: withAlpha(accent, 0.1),
                bottomColor: withAlpha(accent, 0.1),
                crosshairMarkerBorderColor: tokenColor(element, '--surface'),
                crosshairMarkerBackgroundColor: accent,
            });
            newcomers.current?.applyOptions({ color: tokenColor(element, '--ink-2') });
            played.current?.applyOptions({ color: tokenColor(element, '--chart-muted') });
        };
        paint();
        const theme = new MutationObserver(paint);
        theme.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
        const hover = (event: MouseEventParams<Time>) => {
            const point = shown.current.find((each) => each.day === event.time);
            if (event.point === undefined || point === undefined) {
                setHovered(null);
                return;
            }
            setHovered({ x: event.point.x + api.priceScale('left').width(), point });
        };
        api.subscribeCrosshairMove(hover);
        return () => {
            theme.disconnect();
            api.unsubscribeCrosshairMove(hover);
            api.remove();
            chart.current = null;
            players.current = null;
            newcomers.current = null;
            played.current = null;
        };
    }, []);

    useEffect(() => {
        localeRef.current = locale;
        const element = host.current;
        if (element !== null) {
            chart.current?.applyOptions(dayChartOptions(element, locale));
        }
    }, [locale]);

    // A week before the last two has no count of unique players: a gap, never a zero.
    useEffect(() => {
        players.current?.setData(
            points.map((point) =>
                point.players === null ? { time: point.day } : { time: point.day, value: point.players },
            ),
        );
        newcomers.current?.setData(points.map((point) => ({ time: point.day, value: point.newPlayers })));
        played.current?.setData(
            points.map((point) => ({ time: point.day, value: point.playMs / 3_600_000 })),
        );
        chart.current?.timeScale().fitContent();
    }, [points]);

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

    const words = t.players.community;
    return (
        <div className="relative h-[260px] max-md:h-[200px]" aria-hidden="true">
            <div ref={host} className="absolute inset-0" />
            {hovered && (
                <span
                    ref={tip}
                    className="pointer-events-none absolute top-2 flex -translate-x-1/2 flex-col rounded-sm border border-line-strong bg-surface px-2.5 py-1.5 whitespace-nowrap shadow-pop"
                >
                    <span className="text-ui font-semibold text-ink">
                        {words.tooltipPlayers(hovered.point.players)}
                    </span>
                    <span className="text-small text-ink-2">
                        {words.tooltipOthers(hovered.point.newPlayers, format.playTime(hovered.point.playMs))}
                    </span>
                    <span className="font-mono text-meta text-ink-3">
                        {weekly
                            ? words.weekOf(hovered.point.day, dayBefore(hovered.point.day, -6))
                            : hovered.point.day}
                    </span>
                    {hovered.point.today && (
                        <span className="text-meta text-ink-3">
                            {weekly ? words.weekSoFar : words.todaySoFar}
                        </span>
                    )}
                </span>
            )}
        </div>
    );
}
