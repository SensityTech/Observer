import { describe, expect, it } from 'vitest';

import type { LiveSample } from '../api/generated/LiveSample';
import type { MetricsSeries } from '../api/generated/MetricsSeries';
import { keepLive, mergeLive } from './merge';

const sample = (ts: number, values: LiveSample['values']): LiveSample => ({
    ts,
    state: 'online',
    values,
    children: [],
    cores: 8,
});

/** Four buckets of 30 s from 1,000,000. */
const grouped = (): MetricsSeries => ({
    range: '6h',
    step: 30,
    from: 1_000_000,
    buckets: 4,
    last: 1_000_095,
    series: [
        {
            series: 'players',
            values: [10, null, null, 20],
            min: [10, null, null, 20],
            max: [10, null, null, 20],
            samples: [6, 0, 0, 1],
        },
        { series: 'joins', values: [1, null, null, 2] },
    ],
    markers: [],
    observed: [6, 0, 0, 1],
    online: [6, 0, 0, 1],
});

/** An hour of 5 s buckets from 1,000,000, with no joins in any but the open one. */
const quietHour = (): MetricsSeries => ({
    range: '1h',
    step: 5,
    from: 1_000_000,
    buckets: 720,
    last: 1_000_000 + 718 * 5,
    series: [
        { series: 'joins', values: Array.from({ length: 720 }, (_, index) => (index < 719 ? 0 : null)) },
    ],
    markers: [],
    observed: Array.from({ length: 720 }, (_, index) => (index < 719 ? 1 : 0)),
    online: Array.from({ length: 720 }, (_, index) => (index < 719 ? 1 : 0)),
});

describe('keepLive', () => {
    it('should hold every sample a page read five minutes ago lacks', () => {
        const page = quietHour();
        let kept: LiveSample[] = [];
        for (let slot = 1; slot <= 60; slot += 1) {
            kept = keepLive(kept, sample((page.last ?? 0) + slot * 5, { joins: 1 }));
        }

        const joins = mergeLive(page, kept).series[0]?.values ?? [];
        expect(joins.slice(-60)).toEqual(Array<number>(60).fill(1));
        expect(joins.reduce<number>((sum, value) => sum + (value ?? 0), 0)).toBe(60);
    });

    it('should let a sample go once it is six minutes older than the newest', () => {
        let kept: LiveSample[] = [];
        for (let slot = 1; slot <= 100; slot += 1) {
            kept = keepLive(kept, sample(1_000_000 + slot * 5, {}));
        }

        expect(kept).toHaveLength(72);
        expect(kept[0]?.ts).toBe(1_000_000 + 29 * 5);
    });

    it('should replace a sample it already holds', () => {
        const kept = keepLive([sample(1_000_000, { joins: 1 })], sample(1_000_000, { joins: 2 }));

        expect(kept.map((held) => held.values.joins)).toEqual([2]);
    });
});

describe('mergeLive', () => {
    it('should fold a sample into the open bucket of a grouped range', () => {
        const merged = mergeLive(grouped(), [sample(1_000_100, { players: 26, joins: 3 })]);

        const [players, joins] = merged.series;
        expect(players?.values[3]).toBe(23);
        expect(players?.samples?.[3]).toBe(2);
        expect([players?.min?.[3], players?.max?.[3]]).toEqual([20, 26]);
        expect(joins?.values[3]).toBe(5);
        expect(merged.last).toBe(1_000_100);
    });

    it('should open the next bucket past the grid and move the grid on', () => {
        const merged = mergeLive(grouped(), [sample(1_000_125, { players: 30 })]);

        expect(merged.from).toBe(1_000_030);
        expect(merged.series[0]?.values).toEqual([null, null, 20, 30]);
        expect(merged.series[0]?.samples).toEqual([0, 0, 1, 1]);
        expect(merged.series[1]?.values).toEqual([null, null, 2, null]);
    });

    it('should ignore what the page already holds and leave a week or a month as read', () => {
        const page = grouped();

        expect(mergeLive(page, [sample(1_000_095, { players: 99 })])).toEqual(page);
        expect(mergeLive({ ...page, range: '7d' }, [sample(1_000_200, { players: 99 })]).series).toEqual(
            page.series,
        );
    });

    it('should take each sample as its own bucket over an hour', () => {
        const hour: MetricsSeries = {
            range: '1h',
            step: 5,
            from: 1_000_000,
            buckets: 3,
            last: null,
            series: [{ series: 'fxsCpu', values: [null, null, null] }],
            markers: [],
            observed: [0, 0, 0],
            online: [0, 0, 0],
        };

        const merged = mergeLive(hour, [
            sample(1_000_005, { fxsCpu: 182 }),
            sample(1_000_015, { fxsCpu: 90 }),
        ]);

        expect(merged.from).toBe(1_000_005);
        expect(merged.series[0]?.values).toEqual([182, null, 90]);
    });
});

describe('mergeLive coverage', () => {
    it('should count each sample in its bucket, and among the online ones when it was', () => {
        const merged = mergeLive(grouped(), [
            sample(1_000_100, {}),
            { ...sample(1_000_105, {}), state: 'starting' },
            { ...sample(1_000_110, {}), state: 'degraded' },
        ]);

        expect(merged.observed).toEqual([6, 0, 0, 4]);
        expect(merged.online).toEqual([6, 0, 0, 3]);
    });

    it('should move the coverage on with the grid', () => {
        const merged = mergeLive(grouped(), [{ ...sample(1_000_125, {}), state: 'stopped' }]);

        expect(merged.observed).toEqual([0, 0, 1, 1]);
        expect(merged.online).toEqual([0, 0, 1, 0]);
    });

    it('should add up the slot counts of faults, processes started and disk transfers', () => {
        const counts: MetricsSeries = {
            ...grouped(),
            series: [
                { series: 'fxsFaults', values: [10, null, null, 100] },
                { series: 'treeStarted', values: [null, null, null, 1] },
                { series: 'hostDiskTransfers', values: [null, null, null, 40] },
                { series: 'fxsHardFaults', values: [null, null, null, null] },
            ],
        };

        const merged = mergeLive(counts, [
            sample(1_000_100, { fxsFaults: 50, treeStarted: 2, hostDiskTransfers: 9, fxsHardFaults: 1 }),
        ]);

        expect(merged.series.map((data) => data.values[3])).toEqual([150, 3, 49, 1]);
    });
});
