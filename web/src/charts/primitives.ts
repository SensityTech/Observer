import type {
    IPrimitivePaneRenderer,
    IPrimitivePaneView,
    ISeriesPrimitive,
    SeriesAttachedParameter,
    SeriesType,
    Time,
    UTCTimestamp,
} from 'lightweight-charts';

import type { Period } from './coverage';

type Target = Parameters<IPrimitivePaneRenderer['draw']>[0];
type Attached = SeriesAttachedParameter<Time, SeriesType>;

/** A down band never narrower than this, in CSS pixels, so a two-minute stop shows on a day. */
const NARROWEST = 6;

/** The lowest and highest value of a bucket, at its start. */
export type Extremes = { time: number; min: number | null; max: number | null };

/**
 * The band between the buckets' lowest and highest values, under the series' line. It breaks
 * where either is unknown, as the line does.
 */
export class RangeBand implements ISeriesPrimitive<Time> {
    private chart: Attached | null = null;
    private points: readonly Extremes[] = [];
    private color = 'transparent';
    private runs: { x: number; low: number; high: number }[][] = [];
    private readonly view: IPrimitivePaneView = {
        zOrder: () => 'bottom',
        renderer: () => ({ draw: (target: Target) => this.draw(target) }),
    };

    set(points: readonly Extremes[], color: string) {
        this.points = points;
        this.color = color;
        this.chart?.requestUpdate();
    }

    attached(param: Attached) {
        this.chart = param;
    }

    detached() {
        this.chart = null;
    }

    updateAllViews() {
        const attached = this.chart;
        this.runs = [];
        if (attached === null) {
            return;
        }
        let run: { x: number; low: number; high: number }[] = [];
        for (const point of this.points) {
            const x = attached.chart.timeScale().timeToCoordinate(point.time as UTCTimestamp);
            const low = point.min === null ? null : attached.series.priceToCoordinate(point.min);
            const high = point.max === null ? null : attached.series.priceToCoordinate(point.max);
            if (x === null || low === null || high === null) {
                if (run.length > 1) {
                    this.runs.push(run);
                }
                run = [];
                continue;
            }
            run.push({ x, low, high });
        }
        if (run.length > 1) {
            this.runs.push(run);
        }
    }

    paneViews() {
        return [this.view];
    }

    private draw(target: Target) {
        target.useBitmapCoordinateSpace(
            ({ context, horizontalPixelRatio: across, verticalPixelRatio: down }) => {
                context.fillStyle = this.color;
                for (const run of this.runs) {
                    context.beginPath();
                    for (const [index, point] of run.entries()) {
                        if (index === 0) {
                            context.moveTo(point.x * across, point.high * down);
                        } else {
                            context.lineTo(point.x * across, point.high * down);
                        }
                    }
                    for (const point of run.toReversed()) {
                        context.lineTo(point.x * across, point.low * down);
                    }
                    context.closePath();
                    context.fill();
                }
            },
        );
    }
}

/** Full-height bands over the stretches the server was down, edged on both sides. */
export class DownBands implements ISeriesPrimitive<Time> {
    private chart: Attached | null = null;
    private periods: readonly Period[] = [];
    private step = 0;
    private colors = { fill: 'transparent', edge: 'transparent' };
    private spans: [number, number][] = [];
    private readonly view: IPrimitivePaneView = {
        zOrder: () => 'bottom',
        renderer: () => ({ draw: (target: Target) => this.draw(target) }),
    };

    set(periods: readonly Period[], step: number, colors: { fill: string; edge: string }) {
        this.periods = periods;
        this.step = step;
        this.colors = colors;
        this.chart?.requestUpdate();
    }

    attached(param: Attached) {
        this.chart = param;
    }

    detached() {
        this.chart = null;
    }

    updateAllViews() {
        const attached = this.chart;
        this.spans = [];
        if (attached === null) {
            return;
        }
        const x = (time: number) => attached.chart.timeScale().timeToCoordinate(time as UTCTimestamp);
        for (const period of this.periods) {
            const first = x(period.from);
            const last = x(period.to - this.step);
            if (first === null || last === null) {
                continue;
            }
            // A bucket is as wide as the distance to the next one.
            const next = x(period.to);
            const before = x(period.to - 2 * this.step);
            const width = next === null ? (before === null ? 0 : last - before) : next - last;
            let start: number = first;
            let end: number = last + width;
            if (end - start < NARROWEST) {
                const middle = (start + end) / 2;
                start = middle - NARROWEST / 2;
                end = middle + NARROWEST / 2;
            }
            this.spans.push([start, end]);
        }
    }

    paneViews() {
        return [this.view];
    }

    private draw(target: Target) {
        target.useBitmapCoordinateSpace(({ context, bitmapSize, horizontalPixelRatio: across }) => {
            const edge = Math.max(1, Math.round(across));
            for (const [start, end] of this.spans) {
                const left = Math.round(start * across);
                const right = Math.round(end * across);
                context.fillStyle = this.colors.fill;
                context.fillRect(left, 0, right - left, bitmapSize.height);
                context.fillStyle = this.colors.edge;
                context.fillRect(left, 0, edge, bitmapSize.height);
                context.fillRect(right - edge, 0, edge, bitmapSize.height);
            }
        });
    }
}
