import type { LiveSample } from '../api/generated/LiveSample';
import type { MetricsSeries } from '../api/generated/MetricsSeries';
import type { Series } from '../api/generated/Series';
import type { SeriesData } from '../api/generated/SeriesData';
import { REFETCH_EVERY } from '../api/metrics';

/** As long as a page they complete may be old: until it is read again, and a minute more. */
const KEPT_SECONDS = REFETCH_EVERY / 1000 + 60;

/** Series whose bucket holds the slot's total rather than an average. */
const COUNTS = new Set<Series>(['joins', 'leaves', 'hitchesMain', 'hitchesNetwork', 'hitchesSync', 'barks']);

/**
 * The live samples held once `sample` came, oldest first. A sample already held, a snapshot after
 * a reconnect say, replaces itself rather than repeat.
 */
export const keepLive = (kept: readonly LiveSample[], sample: LiveSample): LiveSample[] => [
    ...kept.filter((held) => held.ts < sample.ts && held.ts > sample.ts - KEPT_SECONDS),
    sample,
];

/**
 * A page of series with the live samples taken after its newest one. A sample opens the next
 * bucket when it falls past the grid, which moves on by as many buckets. A week or a month shows
 * closed minutes and hours only, so they are left as read.
 */
export const mergeLive = (page: MetricsSeries, samples: readonly LiveSample[]): MetricsSeries => {
    if (page.range === '7d' || page.range === '30d') {
        return page;
    }
    let merged = page;
    for (const sample of samples) {
        if (merged.last === null || sample.ts > merged.last) {
            merged = merge(merged, sample);
        }
    }
    return merged;
};

const merge = (page: MetricsSeries, sample: LiveSample): MetricsSeries => {
    let index = Math.floor((sample.ts - page.from) / page.step);
    if (index < 0) {
        return page;
    }
    const shift = Math.max(0, index - (page.buckets - 1));
    index -= shift;
    const series = page.series.map((data) => {
        const moved = shift > 0 ? moveOn(data, shift) : data;
        const value = sample.values[data.series];
        return value === undefined ? moved : add(moved, index, value, page.range === '1h');
    });
    return { ...page, from: page.from + shift * page.step, series, last: sample.ts };
};

const moveOn = (data: SeriesData, buckets: number): SeriesData => {
    const along = <T>(values: T[], empty: T) => [
        ...values.slice(buckets),
        ...Array<T>(Math.min(buckets, values.length)).fill(empty),
    ];
    return {
        ...data,
        values: along(data.values, null),
        ...(data.min && { min: along(data.min, null) }),
        ...(data.max && { max: along(data.max, null) }),
        ...(data.samples && { samples: along(data.samples, 0) }),
    };
};

const add = (data: SeriesData, index: number, value: number, slots: boolean): SeriesData => {
    const values = [...data.values];
    const held = values[index] ?? null;
    if (COUNTS.has(data.series)) {
        values[index] = (held ?? 0) + value;
        return { ...data, values };
    }
    // An hour's bucket is one slot: the sample is the bucket. A derived series of a longer range
    // comes without the totals it was computed from, so the sample only fills an empty bucket.
    if (slots || data.samples === undefined || data.min === undefined || data.max === undefined) {
        values[index] = slots ? value : (held ?? value);
        return { ...data, values };
    }
    const samples = [...data.samples];
    const min = [...data.min];
    const max = [...data.max];
    const count = samples[index] ?? 0;
    values[index] = held === null || count === 0 ? value : (held * count + value) / (count + 1);
    samples[index] = count + 1;
    min[index] = Math.min(min[index] ?? value, value);
    max[index] = Math.max(max[index] ?? value, value);
    return { ...data, values, samples, min, max };
};
