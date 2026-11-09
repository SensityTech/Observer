import {
    HistogramSeries,
    createChart,
    type AutoscaleInfo,
    type ChartOptions,
    type DeepPartial,
    type IChartApi,
    type ISeriesApi,
    type MouseEventParams,
    type Time,
} from 'lightweight-charts';
import { useEffect, useLayoutEffect, useRef, useState } from 'react';

import { formatPlayTime } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { dayChartOptions } from './days';
import { tokenColor } from './theme';

const HOUR = 3_600_000;

/**
 * The scale spans an hour at least, so that a few minutes played still read on distinct marks
 * (`12 min`, `24 min`…) rather than on several `0h`.
 */
export const atLeastAnHour = (info: AutoscaleInfo | null): AutoscaleInfo => ({
    ...info,
    priceRange: { minValue: 0, maxValue: Math.max(1, info?.priceRange?.maxValue ?? 0) },
});

/** The scale's marks, in the words play time is written in everywhere else: `24 min`, `1h 30m`. */
const activityOptions = (host: Element, locale: string): DeepPartial<ChartOptions> => {
    const base = dayChartOptions(host, locale);
    return {
        ...base,
        // The library sizes the scale on its first and last marks only (`0h`, `1h`): `48 min` needs more.
        leftPriceScale: { ...base.leftPriceScale, minimumWidth: 56 },
        localization: {
            ...base.localization,
            priceFormatter: (hours: number) => formatPlayTime(locale, hours * HOUR),
        },
    };
};

/** A player's play time per day as grey bars, today's included as it stands. */
export function ActivityChart({ days, today }: { days: { day: string; playMs: number }[]; today: string }) {
    const { t, format, locale } = useI18n();
    const host = useRef<HTMLDivElement>(null);
    const chart = useRef<IChartApi | null>(null);
    const bars = useRef<ISeriesApi<'Histogram'> | null>(null);
    const localeRef = useRef({ locale, t });
    const shown = useRef(days);
    const [hovered, setHovered] = useState<{ x: number; day: string; playMs: number } | null>(null);
    const tip = useRef<HTMLSpanElement>(null);

    useEffect(() => {
        shown.current = days;
    }, [days]);

    useEffect(() => {
        const element = host.current;
        if (element === null) {
            return;
        }
        const api = createChart(element, activityOptions(element, localeRef.current.locale));
        bars.current = api.addSeries(HistogramSeries, {
            priceScaleId: 'left',
            priceLineVisible: false,
            lastValueVisible: false,
            autoscaleInfoProvider: atLeastAnHour,
        });
        chart.current = api;
        const paint = () => {
            api.applyOptions(activityOptions(element, localeRef.current.locale));
            bars.current?.applyOptions({ color: tokenColor(element, '--chart-muted') });
        };
        paint();
        const theme = new MutationObserver(paint);
        theme.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
        const hover = (event: MouseEventParams<Time>) => {
            const day = shown.current.find((each) => each.day === event.time);
            if (event.point === undefined || day === undefined) {
                setHovered(null);
                return;
            }
            setHovered({ x: event.point.x + api.priceScale('left').width(), ...day });
        };
        api.subscribeCrosshairMove(hover);
        return () => {
            theme.disconnect();
            api.unsubscribeCrosshairMove(hover);
            api.remove();
            chart.current = null;
            bars.current = null;
        };
    }, []);

    useEffect(() => {
        localeRef.current = { locale, t };
        const element = host.current;
        if (element !== null) {
            chart.current?.applyOptions(activityOptions(element, locale));
        }
    }, [locale, t]);

    useEffect(() => {
        bars.current?.setData(days.map((each) => ({ time: each.day, value: each.playMs / HOUR })));
        chart.current?.timeScale().fitContent();
    }, [days]);

    useLayoutEffect(() => {
        const element = tip.current;
        const plot = host.current;
        if (hovered === null || element === null || plot === null) {
            return;
        }
        const half = element.offsetWidth / 2;
        element.style.left = `${Math.max(half, Math.min(hovered.x, plot.clientWidth - half))}px`;
    }, [hovered]);

    const words = t.players.profile.activity;
    return (
        <div className="relative h-[200px]" aria-hidden="true">
            <div ref={host} className="absolute inset-0" />
            {hovered && (
                <span
                    ref={tip}
                    className="pointer-events-none absolute top-2 flex -translate-x-1/2 flex-col rounded-sm border border-line-strong bg-surface px-2.5 py-1.5 whitespace-nowrap shadow-pop"
                >
                    <span className="text-ui font-semibold text-ink">
                        {words.tooltip(format.playTime(hovered.playMs))}
                    </span>
                    <span className="font-mono text-meta text-ink-3">{hovered.day}</span>
                    {hovered.day === today && (
                        <span className="text-meta text-ink-3">{words.todaySoFar}</span>
                    )}
                </span>
            )}
        </div>
    );
}
