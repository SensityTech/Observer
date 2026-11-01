import type {
    IPrimitivePaneRenderer,
    ITimeScaleApi,
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

/** A band from `start` to `end`, widened about its middle to the narrowest a band is drawn. */
const widened = (start: number, end: number): [number, number] => {
    if (end - start >= NARROWEST) {
        return [start, end];
    }
    const middle = (start + end) / 2;
    return [middle - NARROWEST / 2, middle + NARROWEST / 2];
};

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

/** A bucket's start as the chart holds it: an instant by default. */
const asInstant = (seconds: number): Time => seconds as UTCTimestamp;

/**
 * Where an instant falls on a time scale whose points are the buckets' `starts`: between its
 * bucket's start and the next one's, which need not be evenly spaced (a day of a time zone across a
 * change of time is 23 or 25 hours). The last bucket lasts `step` seconds, as wide as the one before.
 * `timeOf` gives a start as the chart holds it, a calendar day for a chart of days. `null` before
 * the first bucket or off the scale.
 */
export const instantX = (
    timeScale: ITimeScaleApi<Time>,
    starts: readonly number[],
    step: number,
    time: number,
    timeOf: (seconds: number) => Time = asInstant,
) => {
    const index = starts.findLastIndex((start) => start <= time);
    const start = starts[index];
    if (start === undefined) {
        return null;
    }
    const x = (at: number) => timeScale.timeToCoordinate(timeOf(at));
    const left = x(start);
    if (left === null) {
        return null;
    }
    const next = starts[index + 1];
    if (next !== undefined) {
        const right = x(next);
        return right === null ? left : left + ((time - start) / (next - start)) * (right - left);
    }
    const before = starts[index - 1];
    const previous = before === undefined ? null : x(before);
    const width = previous === null ? 0 : left - previous;
    return left + Math.min(1, (time - start) / step) * width;
};

/**
 * Full-height bands over the stretches the server was down, edged on both sides. Periods are whole
 * buckets; given the buckets' `starts`, they are instants placed inside their buckets, so a crash of
 * two minutes is not drawn as an hour down.
 */
export class DownBands implements ISeriesPrimitive<Time> {
    private chart: Attached | null = null;
    private periods: readonly Period[] = [];
    private step = 0;
    private starts: readonly number[] | null = null;
    private timeOf: (seconds: number) => Time = asInstant;
    private colors = { fill: 'transparent', edge: 'transparent' };
    private spans: [number, number][] = [];
    private readonly view: IPrimitivePaneView = {
        zOrder: () => 'bottom',
        renderer: () => ({ draw: (target: Target) => this.draw(target) }),
    };

    set(
        periods: readonly Period[],
        step: number,
        colors: { fill: string; edge: string },
        starts?: readonly number[],
        timeOf: (seconds: number) => Time = asInstant,
    ) {
        this.periods = periods;
        this.step = step;
        this.starts = starts ?? null;
        this.timeOf = timeOf;
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
        const starts = this.starts;
        for (const period of this.periods) {
            if (starts !== null) {
                const from = instantX(
                    attached.chart.timeScale(),
                    starts,
                    this.step,
                    period.from,
                    this.timeOf,
                );
                const to = instantX(attached.chart.timeScale(), starts, this.step, period.to, this.timeOf);
                if (from !== null && to !== null) {
                    this.spans.push(widened(from, to));
                }
                continue;
            }
            const first = x(period.from);
            const last = x(period.to - this.step);
            if (first === null || last === null) {
                continue;
            }
            // A bucket is as wide as the distance to the next one.
            const next = x(period.to);
            const before = x(period.to - 2 * this.step);
            const width = next === null ? (before === null ? 0 : last - before) : next - last;
            this.spans.push(widened(first, last + width));
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
