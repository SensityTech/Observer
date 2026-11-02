import type { Growth } from '../api/generated/Growth';
import type { Series } from '../api/generated/Series';
import type { Timeline } from '../api/generated/Timeline';
import type { Extremes } from '../charts/primitives';
import type { ExtraLane } from './search';

/** A lane of the timeline: the four it always draws, then those a viewer adds. */
export type LaneKey = 'players' | 'frames' | 'cpu' | 'memory' | ExtraLane;

export const BASE_LANES: readonly LaneKey[] = ['players', 'frames', 'cpu', 'memory'];

/** The series each lane reads. */
export const LANE_SERIES = {
    players: ['players'],
    frames: ['mainTick', 'hitchesMain', 'hitchMax'],
    cpu: [
        'fxsCpu',
        'mainThreadCpu',
        'networkThreadCpu',
        'syncThreadCpu',
        'jsThreadsCpu',
        'monoThreadsCpu',
        'otherThreadsCpu',
    ],
    memory: ['fxsPrivate'],
    ping: ['pingAvg', 'pingP95'],
    network: ['hostReceive', 'hostSend'],
    heap: ['jsHeapUsed', 'jsHeapTotal'],
    disk: ['hostDiskLatency'],
} as const satisfies Record<LaneKey, readonly Series[]>;

/** svMain's budget, as `perf::BUDGET` has it. */
export const FRAME_BUDGET_MS = 50;
/** How far past now the memory forecast reaches, as a share of the window. */
export const FORECAST_SHARE = 0.125;
const WEEK = 7 * 86_400;

/** The series a set of lanes reads, each once, in the lanes' order. */
export const seriesOf = (lanes: readonly LaneKey[]): Series[] => [
    ...new Set(lanes.flatMap((lane) => LANE_SERIES[lane])),
];

export type Row = { time: number; value: number | null };

export const valuesOf = (timeline: Timeline, series: Series): (number | null)[] =>
    timeline.series.find((data) => data.series === series)?.values ?? [];

export const timeOf = (timeline: Timeline, index: number) => timeline.from + index * timeline.step;

/** A series' buckets as rows, moved by `shift` seconds: the week before is drawn over this one. */
export const rowsOf = (timeline: Timeline, series: Series, shift = 0): Row[] =>
    valuesOf(timeline, series).map((value, index) => ({ time: timeOf(timeline, index) + shift, value }));

/** The bucket that holds `time`, within the grid. */
export const bucketAt = (timeline: Timeline, time: number) =>
    Math.max(0, Math.min(timeline.buckets - 1, Math.floor((time - timeline.from) / timeline.step)));

/** The newest bucket with a value of `series`, or the last bucket. */
export const newestBucket = (timeline: Timeline, series: Series) => {
    const index = valuesOf(timeline, series).findLastIndex((value) => value !== null);
    return index === -1 ? timeline.buckets - 1 : index;
};

const sum = (values: readonly (number | null | undefined)[]) =>
    values.some((value) => value === null || value === undefined)
        ? null
        : values.reduce<number>((total, value) => total + (value ?? 0), 0);

/**
 * The CPU lane's areas, each the sum of the groups under it: svMain, then svNetwork and svSync on
 * top, then every other thread. The groups are measured together, so a bucket has all or none.
 */
export const cpuStack = (timeline: Timeline) => {
    const group = (series: Series) => valuesOf(timeline, series);
    const [main, network, sync, js, mono, other] = [
        group('mainThreadCpu'),
        group('networkThreadCpu'),
        group('syncThreadCpu'),
        group('jsThreadsCpu'),
        group('monoThreadsCpu'),
        group('otherThreadsCpu'),
    ];
    const rows = (pick: (index: number) => number | null): Row[] =>
        Array.from({ length: timeline.buckets }, (_, index) => ({
            time: timeOf(timeline, index),
            value: pick(index),
        }));
    return {
        main: rows((index) => main[index] ?? null),
        server: rows((index) => sum([main[index], network[index], sync[index]])),
        all: rows((index) =>
            sum([main[index], network[index], sync[index], js[index], mono[index], other[index]]),
        ),
    };
};

/**
 * The frame time lane's line: the median where the window has percentiles, the mean of every
 * tick otherwise, before the histograms were kept.
 */
export const frameLine = (timeline: Timeline): { kind: 'median' | 'mean'; rows: Row[] } => {
    const p50 = timeline.frames?.p50 ?? [];
    if (p50.some((value) => value !== null)) {
        return {
            kind: 'median',
            rows: p50.map((value, index) => ({ time: timeOf(timeline, index), value })),
        };
    }
    return { kind: 'mean', rows: rowsOf(timeline, 'mainTick') };
};

/** The band from p95 to p99 of each bucket. */
export const frameBand = (timeline: Timeline): Extremes[] =>
    (timeline.frames?.p95 ?? []).map((low, index) => ({
        time: timeOf(timeline, index),
        min: low,
        max: timeline.frames?.p99[index] ?? null,
    }));

/** The buckets with a hitch of svMain, and the longest one in each. */
export const hitches = (timeline: Timeline) => {
    const counts = valuesOf(timeline, 'hitchesMain');
    const longest = valuesOf(timeline, 'hitchMax');
    return counts.flatMap((count, index) =>
        count !== null && count > 0
            ? [{ time: timeOf(timeline, index), count, longest: longest[index] ?? null }]
            : [],
    );
};

/** Where players usually were at the same time of the 4 weeks before. */
export const usualBand = (timeline: Timeline): Extremes[] =>
    (timeline.usual?.low ?? []).map((low, index) => ({
        time: timeOf(timeline, index),
        min: low,
        max: timeline.usual?.high[index] ?? null,
    }));

/** The week before, drawn over this one: its times moved a week later. */
export const shiftedRows = (compare: Timeline, series: Series) => rowsOf(compare, series, WEEK);

/** The week before's total CPU, moved a week later. */
export const shiftedCpu = (compare: Timeline) =>
    cpuStack(compare).all.map((row) => ({ time: row.time + WEEK, value: row.value }));

/**
 * The open run's growth as a line from `start` (its first fitted minute, or later) to `until`, and
 * the band a new minute would fall in from `now` on, every `step` seconds. Times are Unix seconds;
 * the growth's `from` is too.
 */
export const forecast = (
    growth: Growth,
    from: number,
    now: number,
    until: number,
    step: number,
    start = from,
) => {
    const at = (time: number) => {
        const x = time - from;
        const mid = growth.start + (growth.perHour / 3600) * x;
        if (growth.residual === null || growth.slopeError === null) {
            return { mid, low: null, high: null };
        }
        const away = x - growth.center;
        const slope = growth.slopeError / 3600;
        const half = 1.96 * Math.sqrt(growth.residual ** 2 + away ** 2 * slope ** 2);
        return { mid, low: mid - half, high: mid + half };
    };
    const times = (first: number, last: number) => {
        const out: number[] = [];
        for (let time = first; time < last; time += step) {
            out.push(time);
        }
        out.push(last);
        return out;
    };
    const begin = Math.ceil(Math.max(from, start) / step) * step;
    const trend: Row[] = times(begin, until).map((time) => ({ time, value: at(time).mid }));
    const cone: Extremes[] = times(now, until).map((time) => {
        const { low, high } = at(time);
        return { time, min: low, max: high };
    });
    return { trend, cone };
};

/** The change of `values` over the `seconds` before bucket `index`, when both ends were measured. */
export const changeOver = (timeline: Timeline, series: Series, index: number, seconds: number) => {
    const values = valuesOf(timeline, series);
    const back = Math.max(1, Math.round(seconds / timeline.step));
    const now = values[index] ?? null;
    const before = values[index - back] ?? null;
    return now === null || before === null ? null : now - before;
};
