import type { AreaData, HistogramData, LineData, Time, WhitespaceData } from 'lightweight-charts';

import type { DropSeries } from '../api/generated/DropSeries';
import type { RunMarker } from '../api/generated/RunMarker';
import type { Period } from '../charts/coverage';
import { pointOf } from './times';

/**
 * A bucket of the chart: when it starts, in Unix seconds, and where the chart puts it (an instant,
 * or a calendar day of the server's zone), its rate and baseline per 100 player-hours, its drops.
 */
export type DropBucket = {
    time: number;
    point: Time;
    rate: number | null;
    baseline: number | null;
    drops: number;
};

/**
 * The series as the chart draws it, each bucket at the start the API gives it: a day of the
 * schedule's zone across a change of time is 23 or 25 hours, so starts are not `from + i × step`.
 */
export const dropBuckets = (series: DropSeries, zone: string): DropBucket[] =>
    series.rates.map((rate, index) => {
        const time = series.starts[index] ?? series.from + index * series.step;
        return {
            time,
            point: pointOf(time, series.step, zone),
            rate,
            baseline: series.baseline[index] ?? null,
            drops: series.drops[index] ?? 0,
        };
    });

/**
 * A value per bucket, where `null` is whitespace and the point before it takes `hidden`, the colours
 * that make it transparent: the library would otherwise join both sides of a gap, which must read
 * as a gap, never as zero. The buckets after the newest value are still to come, not a gap.
 */
const gaps = <Point extends { time: Time; value: number }>(
    rows: readonly DropBucket[],
    value: (row: DropBucket) => number | null,
    hidden: Omit<Point, 'time' | 'value'>,
): (Point | WhitespaceData)[] => {
    const newest = rows.findLastIndex((row) => value(row) !== null);
    return rows.map((row, index) => {
        const measured = value(row);
        if (measured === null) {
            return { time: row.point };
        }
        const next = rows[index + 1];
        const beforeGap = index < newest && next !== undefined && value(next) === null;
        return (
            beforeGap ? { time: row.point, value: measured, ...hidden } : { time: row.point, value: measured }
        ) as Point;
    });
};

/**
 * The rate as the main area. Nobody played in a bucket whose rate is `null`: it is whitespace, and
 * the line breaks there rather than dropping to zero.
 */
export const ratePoints = (rows: readonly DropBucket[], transparent: string): (AreaData | WhitespaceData)[] =>
    gaps<AreaData>(rows, (row) => row.rate, {
        lineColor: transparent,
        topColor: transparent,
        bottomColor: transparent,
    });

/** The 4-week baseline, broken the same way where the weeks before had nobody playing. */
export const baselinePoints = (
    rows: readonly DropBucket[],
    transparent: string,
): (LineData | WhitespaceData)[] => gaps<LineData>(rows, (row) => row.baseline, { color: transparent });

/** The drops counted, as bars; a bucket nobody played in is whitespace, not a bar of zero. */
export const countPoints = (rows: readonly DropBucket[]): (HistogramData | WhitespaceData)[] =>
    rows.map((row) =>
        row.rate === null && row.drops === 0 ? { time: row.point } : { time: row.point, value: row.drops },
    );

/** The 4-week baseline's lowest and highest over the chart, or `null` before four weeks are known. */
export const baselineSpan = (rows: readonly DropBucket[]) => {
    const known = rows.flatMap((row) => (row.baseline === null ? [] : [row.baseline]));
    return known.length === 0 ? null : { low: Math.min(...known), high: Math.max(...known) };
};

const STEPS = [1, 2, 2.5, 5, 10];

/**
 * The top of the rate scale: the next 1, 2, 2.5 or 5 × 10ⁿ above the highest rate or baseline plus
 * 10%, so that neither touches the top. Rates are small numbers: 0.5, 3 or 25 per 100 hours.
 */
export const rateTop = (rows: readonly DropBucket[]) => {
    const most = rows.reduce((top, row) => Math.max(top, row.rate ?? 0, row.baseline ?? 0), 0);
    if (most <= 0) {
        return 1;
    }
    const wanted = most * 1.1;
    const magnitude = 10 ** Math.floor(Math.log10(wanted));
    const step = STEPS.find((each) => each * magnitude >= wanted) ?? 10;
    // 0.1 × 3 is 0.30000000000000004: the scale's top is a round number.
    return Number((step * magnitude).toPrecision(3));
};

/**
 * The stretches FXServer was down after a crash, in Unix seconds within the chart: until it was
 * back online, else until the next start the chart marks, else until the chart's end.
 */
export const crashPeriods = (markers: readonly RunMarker[], from: number, to: number): Period[] => {
    const ordered = markers.toSorted((a, b) => a.at - b.at);
    return ordered.flatMap((marker, index) => {
        if (marker.kind !== 'crash') {
            return [];
        }
        const next = ordered.slice(index + 1).find((later) => later.kind !== 'crash');
        const end = Math.min(marker.backOnlineAt ?? next?.at ?? to, to);
        const start = Math.max(marker.at, from);
        return end > start ? [{ from: start, to: end }] : [];
    });
};

/** Where the chart ends: the last bucket's start, plus a step. */
export const seriesEnd = (series: DropSeries) => (series.starts.at(-1) ?? series.from) + series.step;

/** The bucket with the highest rate, if anyone played at all. */
export const peakOf = (rows: readonly DropBucket[]) =>
    rows.reduce<DropBucket | null>(
        (best, row) => (row.rate !== null && (best === null || row.rate > (best.rate ?? 0)) ? row : best),
        null,
    );
