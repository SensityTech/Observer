import type { MetricsSeries } from '../api/generated/MetricsSeries';
import type { Series } from '../api/generated/Series';

/** Each thread's budget per tick in milliseconds, those the server counts ticks over: svMain runs at 20 Hz. */
export const BUDGETS = { main: 50, network: 10, sync: 8 } as const;

/** The lowest a log scale can show: a tick shorter than this is drawn on it. */
export const FLOOR_MS = 0.01;

export const TICK_SERIES = [
    'mainTick',
    'networkTick',
    'syncTick',
    'mainOverBudget',
    'networkOverBudget',
    'syncOverBudget',
    'hitchMax',
] as const satisfies readonly Series[];

export type TickBucket = { time: number; main: number | null; network: number | null; sync: number | null };

const values = (page: MetricsSeries, series: Series) =>
    page.series.find((data) => data.series === series)?.values ?? [];

export const clamp = (ms: number | null) => (ms === null ? null : Math.max(FLOOR_MS, ms));

/** The three threads' mean tick per bucket, as the log scale draws them. */
export const tickBuckets = (page: MetricsSeries): TickBucket[] => {
    const main = values(page, 'mainTick');
    const network = values(page, 'networkTick');
    const sync = values(page, 'syncTick');
    return Array.from({ length: page.buckets }, (_, index) => ({
        time: page.from + index * page.step,
        main: clamp(main[index] ?? null),
        network: clamp(network[index] ?? null),
        sync: clamp(sync[index] ?? null),
    }));
};

/**
 * The scale's range: from half the lowest tick to half again over the highest, never under the
 * floor, and always past the svMain budget so that its line shows.
 */
export const tickScale = (rows: readonly TickBucket[]) => {
    const measured = rows.flatMap((row) => [row.main, row.network, row.sync]).filter((ms) => ms !== null);
    const lowest = measured.reduce((low, ms) => Math.min(low, ms), Infinity);
    const highest = measured.reduce((high, ms) => Math.max(high, ms), 0);
    return {
        minValue: Math.max(FLOOR_MS, Number.isFinite(lowest) ? lowest / 2 : FLOOR_MS),
        maxValue: Math.max(BUDGETS.main + 10, highest * 1.5),
    };
};

const mean = (list: readonly (number | null)[]) => {
    const measured = list.filter((value) => value !== null);
    return measured.length === 0 ? null : measured.reduce((sum, value) => sum + value, 0) / measured.length;
};

export type TickFigures = {
    /** svMain's mean tick over the measured buckets, in milliseconds. */
    mainAverage: number | null;
    /** The shares of ticks over each thread's budget, in percent. */
    mainOver: number | null;
    networkOver: number | null;
    syncOver: number | null;
    /** The longest hitch of the range and the start of its bucket. */
    hitch: { ms: number; at: number } | null;
};

/** The figures under the tick chart, which also stand for it in words. */
export const tickFigures = (page: MetricsSeries): TickFigures => {
    let hitch: TickFigures['hitch'] = null;
    for (const [index, ms] of values(page, 'hitchMax').entries()) {
        if (ms !== null && (hitch === null || ms > hitch.ms)) {
            hitch = { ms, at: page.from + index * page.step };
        }
    }
    return {
        mainAverage: mean(values(page, 'mainTick')),
        mainOver: mean(values(page, 'mainOverBudget')),
        networkOver: mean(values(page, 'networkOverBudget')),
        syncOver: mean(values(page, 'syncOverBudget')),
        hitch,
    };
};
