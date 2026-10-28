import type { AreaData, LineData, Time, UTCTimestamp, WhitespaceData } from 'lightweight-charts';

import type { MetricsSeries } from '../api/generated/MetricsSeries';

export type Bucket = { time: number; players: number | null; slots: number | null };

/** The page as the chart draws it: players by bucket average, the slots by their bucket maximum. */
export const buckets = (page: MetricsSeries): Bucket[] => {
    const players = page.series.find((data) => data.series === 'players');
    const slots = page.series.find((data) => data.series === 'slots');
    return Array.from({ length: page.buckets }, (_, index) => ({
        time: page.from + index * page.step,
        players: players?.values[index] ?? null,
        slots: slots?.max?.[index] ?? slots?.values[index] ?? null,
    }));
};

/**
 * An unmeasured bucket is whitespace, and the point before it takes `hidden`, the colours that make
 * it transparent: the library would otherwise join both sides of a stop, which must read as a gap,
 * never as zero.
 */
export const gapPoints = <Point extends { time: Time; value: number }>(
    rows: readonly { time: number; value: number | null }[],
    hidden: Omit<Point, 'time' | 'value'>,
): (Point | WhitespaceData)[] => {
    // The buckets after the newest measured one are still to come, not a gap.
    const newest = rows.findLastIndex((row) => row.value !== null);
    return rows.map((row, index) => {
        const time = row.time as UTCTimestamp;
        if (row.value === null) {
            return { time };
        }
        const beforeGap = index < newest && rows[index + 1]?.value === null;
        return (beforeGap ? { time, value: row.value, ...hidden } : { time, value: row.value }) as Point;
    });
};

export const playerPoints = (rows: Bucket[], transparent: string): (AreaData | WhitespaceData)[] =>
    gapPoints<AreaData>(
        rows.map((row) => ({ time: row.time, value: row.players })),
        { lineColor: transparent, topColor: transparent, bottomColor: transparent },
    );

export const slotPoints = (rows: Bucket[]): (LineData | WhitespaceData)[] =>
    rows.map((row) =>
        row.slots === null
            ? { time: row.time as UTCTimestamp }
            : { time: row.time as UTCTimestamp, value: row.slots },
    );

/**
 * The newest bucket is measured and the one before it is not: the line picks up after a stop. The
 * point before the stop, drawn while it was the newest, must turn transparent, which only drawing
 * every point again does; a live sample otherwise moves the newest bucket alone.
 */
export const resumes = (rows: Bucket[]) => rows.at(-1)?.players != null && rows.at(-2)?.players === null;
