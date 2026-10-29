import { describe, expect, it } from 'vitest';

import type { MetricsSeries } from '../api/generated/MetricsSeries';
import { FLOOR_MS, clamp, tickBuckets, tickFigures, tickScale } from './tick';

/** Four buckets of 30 s from 1,000,000. */
const page = (series: MetricsSeries['series']): MetricsSeries => ({
    range: '6h',
    step: 30,
    from: 1_000_000,
    buckets: 4,
    last: 1_000_095,
    series,
    markers: [],
    observed: [6, 6, 6, 6],
    online: [6, 6, 6, 6],
});

describe('tick time', () => {
    it('should draw a tick too short for the log scale on its floor and keep gaps as gaps', () => {
        expect(clamp(0)).toBe(FLOOR_MS);
        expect(clamp(0.004)).toBe(FLOOR_MS);
        expect(clamp(6.8)).toBe(6.8);
        expect(clamp(null)).toBeNull();

        const rows = tickBuckets(page([{ series: 'mainTick', values: [0, 6.8, null, 12] }]));
        expect(rows.map((row) => row.main)).toEqual([FLOOR_MS, 6.8, null, 12]);
        expect(rows[3]).toEqual({ time: 1_000_090, main: 12, network: null, sync: null });
    });

    it('should scale from half the lowest tick to past the budget, and past the highest', () => {
        const quiet = tickBuckets(
            page([
                { series: 'mainTick', values: [0.08, 0.1, null, 0.09] },
                { series: 'networkTick', values: [0.02, 0.03, null, 0.02] },
            ]),
        );
        const hitched = tickBuckets(page([{ series: 'mainTick', values: [6, 412, 7, 6] }]));

        expect(tickScale(quiet)).toEqual({ minValue: 0.01, maxValue: 60 });
        expect(tickScale(hitched)).toEqual({ minValue: 3, maxValue: 618 });
        expect(tickScale([])).toEqual({ minValue: FLOOR_MS, maxValue: 60 });
    });

    it('should give the averages and shares of the measured buckets and the longest hitch', () => {
        const figures = tickFigures(
            page([
                { series: 'mainTick', values: [6, 8, null, 7] },
                { series: 'mainOverBudget', values: [0, 1.2, null, 0] },
                { series: 'networkOverBudget', values: [0, 0, null, 0] },
                { series: 'hitchMax', values: [null, 412, null, 120] },
            ]),
        );

        expect(figures.mainAverage).toBe(7);
        expect(figures.mainOver).toBeCloseTo(0.4);
        expect(figures.networkOver).toBe(0);
        expect(figures.syncOver).toBeNull();
        expect(figures.hitch).toEqual({ ms: 412, at: 1_000_030 });
        expect(tickFigures(page([])).hitch).toBeNull();
    });
});
