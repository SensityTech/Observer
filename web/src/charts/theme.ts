import {
    ColorType,
    CrosshairMode,
    LineStyle,
    TickMarkType,
    type ChartOptions,
    type DeepPartial,
    type Time,
} from 'lightweight-charts';

import { formatTimestamp } from '../i18n/format';

let pixel: OffscreenCanvasRenderingContext2D | null = null;

/** A colour token resolved where the chart sits, as rgba(): the canvas cannot read var(). */
export function tokenColor(host: Element, name: string): string {
    const probe = document.createElement('i');
    probe.style.cssText = `position:absolute;visibility:hidden;color:var(${name})`;
    host.append(probe);
    // color-mix() comes back as color(srgb ...), which a 1 × 1 canvas turns into plain channels.
    const computed = getComputedStyle(probe).color;
    probe.remove();
    pixel ??= new OffscreenCanvas(1, 1).getContext('2d', { willReadFrequently: true });
    if (pixel === null) {
        return computed;
    }
    pixel.clearRect(0, 0, 1, 1);
    pixel.fillStyle = computed;
    pixel.fillRect(0, 0, 1, 1);
    const [r = 0, g = 0, b = 0, a = 0] = pixel.getImageData(0, 0, 1, 1).data;
    return `rgba(${r}, ${g}, ${b}, ${Math.round((a / 255) * 1000) / 1000})`;
}

export const withAlpha = (rgba: string, alpha: number) => rgba.replace(/[\d.]+\)$/, `${alpha})`);

const pad = (value: number) => String(value).padStart(2, '0');

/** The library counts in UTC seconds; the panel reads every time in the browser's own zone. */
const local = (time: Time) => new Date((time as number) * 1000);

const DAY = new Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short' });

/** Ticks stay where the library puts them; only their words are local: `18:00`, `28 Oct`. */
const tickMark = (time: Time, type: TickMarkType) => {
    const date = local(time);
    return type === TickMarkType.Time || type === TickMarkType.TimeWithSeconds
        ? `${pad(date.getHours())}:${pad(date.getMinutes())}`
        : DAY.format(date);
};

export function chartOptions(host: Element): DeepPartial<ChartOptions> {
    const color = (name: string) => tokenColor(host, name);
    return {
        autoSize: true,
        layout: {
            background: { type: ColorType.Solid, color: 'transparent' },
            textColor: color('--ink-3'),
            fontFamily: getComputedStyle(host).getPropertyValue('--mono').trim(),
            fontSize: 11,
            attributionLogo: false,
        },
        grid: {
            vertLines: { visible: false },
            horzLines: { color: color('--line'), style: LineStyle.Solid },
        },
        leftPriceScale: { visible: true, borderVisible: false, ticksVisible: false, minimumWidth: 38 },
        rightPriceScale: { visible: false },
        timeScale: {
            borderVisible: true,
            borderColor: color('--line-strong'),
            ticksVisible: true,
            timeVisible: true,
            secondsVisible: false,
            fixLeftEdge: true,
            fixRightEdge: true,
            lockVisibleTimeRangeOnResize: true,
            tickMarkFormatter: tickMark,
        },
        crosshair: {
            mode: CrosshairMode.Magnet,
            vertLine: {
                color: color('--line-strong'),
                width: 1,
                style: LineStyle.Dashed,
                labelBackgroundColor: color('--btn-bg'),
            },
            horzLine: { visible: false, labelVisible: false },
        },
        handleScroll: false,
        handleScale: false,
        localization: {
            locale: 'en-GB',
            timeFormatter: (time: Time) => formatTimestamp(local(time), { seconds: false }),
        },
    };
}
