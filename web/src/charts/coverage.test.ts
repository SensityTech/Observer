import { describe, expect, it } from 'vitest';

import type { MetricsSeries } from '../api/generated/MetricsSeries';
import { downPeriods, downTime } from './coverage';

/** Six buckets of 30 s from 1,000,000, with the samples taken and those online in each. */
const page = (observed: number[], online: number[]): MetricsSeries => ({
    range: '6h',
    step: 30,
    from: 1_000_000,
    buckets: observed.length,
    last: null,
    series: [],
    markers: [],
    observed,
    online,
});

describe('downPeriods', () => {
    it('should join the buckets sampled without the server online into stretches', () => {
        const periods = downPeriods(page([6, 6, 6, 6, 6, 6], [6, 0, 0, 6, 0, 6]));

        expect(periods).toEqual([
            { from: 1_000_030, to: 1_000_090 },
            { from: 1_000_120, to: 1_000_150 },
        ]);
        expect(downTime(periods)).toBe(90);
    });

    it('should count a start that never came online as down', () => {
        expect(downPeriods(page([6, 3, 6], [6, 0, 2]))).toEqual([{ from: 1_000_030, to: 1_000_060 }]);
    });

    it('should leave buckets Observer did not sample as gaps, not as stretches down', () => {
        expect(downPeriods(page([0, 0, 6, 0], [0, 0, 6, 0]))).toEqual([]);
        expect(downPeriods(page([6, 0, 6], [0, 0, 0]))).toEqual([
            { from: 1_000_000, to: 1_000_030 },
            { from: 1_000_060, to: 1_000_090 },
        ]);
    });
});
