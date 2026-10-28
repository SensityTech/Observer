import { describe, expect, it } from 'vitest';

import type { LiveSample } from '../api/generated/LiveSample';
import type { MetricsSeries } from '../api/generated/MetricsSeries';
import { mergeLive } from './merge';

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
        };

        const merged = mergeLive(hour, [
            sample(1_000_005, { fxsCpu: 182 }),
            sample(1_000_015, { fxsCpu: 90 }),
        ]);

        expect(merged.from).toBe(1_000_005);
        expect(merged.series[0]?.values).toEqual([182, null, 90]);
    });
});
