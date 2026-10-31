import { describe, expect, it } from 'vitest';

import { SERIES } from './fixtures';
import {
    baselinePoints,
    countPoints,
    crashPeriods,
    dropBuckets,
    peakOf,
    ratePoints,
    rateTop,
    seriesEnd,
} from './points';

const FROM = SERIES.from;
const HOUR = 3_600;
const at = (index: number) => FROM + index * HOUR;
const CLEAR = { lineColor: 'clear', topColor: 'clear', bottomColor: 'clear' };

describe('dropBuckets', () => {
    it('should start each bucket where the API says, with its rate, baseline and drops', () => {
        expect(dropBuckets(SERIES).slice(0, 3)).toEqual([
            { time: at(0), rate: 3.1, baseline: null, drops: 2 },
            { time: at(1), rate: 4.2, baseline: 3.2, drops: 3 },
            { time: at(2), rate: null, baseline: 3.4, drops: 0 },
        ]);
    });

    it('should keep the days of the server zone across a change of time, not 24 hours apart', () => {
        // Days in Paris from 24 to 27 October 2026: the 25th is 25 hours long, clocks went back.
        const starts = [1_792_792_800, 1_792_879_200, 1_792_969_200, 1_793_055_600];
        const days = dropBuckets({
            ...SERIES,
            range: '30d',
            from: starts[0]!,
            step: 86_400,
            starts,
            rates: [1, 2, 3, 4],
            baseline: [],
            drops: [1, 2, 3, 4],
        });
        expect(days.map((day) => day.time)).toEqual(starts);
        expect(starts[2]! - starts[1]!).toBe(25 * HOUR);
        expect(seriesEnd({ ...SERIES, step: 86_400, starts })).toBe(starts[3]! + 86_400);
    });
});

describe('ratePoints', () => {
    it('should leave an hour nobody played as a gap, never a zero, and break the line before it', () => {
        expect(ratePoints(dropBuckets(SERIES), 'clear')).toEqual([
            { time: at(0), value: 3.1 },
            { time: at(1), value: 4.2, ...CLEAR },
            { time: at(2) },
            { time: at(3), value: 5 },
            { time: at(4), value: 6.4 },
            { time: at(5), value: 4.4 },
        ]);
    });

    it('should keep the newest rate drawn while the hours after it are still to come', () => {
        const rows = dropBuckets({ ...SERIES, rates: [3.1, 4.2, null], baseline: [], drops: [2, 3, 0] });
        expect(ratePoints(rows, 'clear')[1]).toEqual({ time: at(1), value: 4.2 });
    });
});

describe('baselinePoints', () => {
    it('should leave hours without four weeks behind them as gaps', () => {
        const points = baselinePoints(dropBuckets(SERIES), 'clear');
        expect(points[0]).toEqual({ time: at(0) });
        expect(points[1]).toEqual({ time: at(1), value: 3.2 });
    });
});

describe('countPoints', () => {
    it('should draw no bar where nobody played, and a bar of zero where they played without a drop', () => {
        const rows = dropBuckets({ ...SERIES, rates: [0, null], baseline: [], drops: [0, 0] });
        expect(countPoints(rows)).toEqual([{ time: at(0), value: 0 }, { time: at(1) }]);
        expect(countPoints(dropBuckets(SERIES))[4]).toEqual({ time: at(4), value: 6 });
    });
});

describe('rateTop', () => {
    it('should round the scale up to 1, 2, 2.5 or 5 × 10ⁿ with room above the highest point', () => {
        expect(rateTop(dropBuckets(SERIES))).toBe(10);
        expect(rateTop(dropBuckets({ ...SERIES, rates: [0.21], baseline: [0.1], drops: [1] }))).toBe(0.25);
        expect(rateTop(dropBuckets({ ...SERIES, rates: [2.6], baseline: [], drops: [1] }))).toBe(5);
        expect(rateTop(dropBuckets({ ...SERIES, rates: [null, 0], baseline: [], drops: [0, 0] }))).toBe(1);
    });
});

describe('peakOf', () => {
    it('should find the highest rate, skipping the gaps', () => {
        expect(peakOf(dropBuckets(SERIES))?.time).toBe(at(4));
        expect(peakOf(dropBuckets({ ...SERIES, rates: [null], baseline: [], drops: [0] }))).toBeNull();
    });
});

describe('crashPeriods', () => {
    it('should keep a crash down until it was back online, cut to the chart', () => {
        const crash = SERIES.markers[0]!;
        expect(crashPeriods(SERIES.markers, FROM, at(6))).toEqual([
            { from: crash.at, to: crash.backOnlineAt },
        ]);
    });

    it('should end a crash without a way back at the next start, or else at the chart end', () => {
        const markers = [
            { at: at(1), kind: 'crash' as const, backOnlineAt: null },
            { at: at(2), kind: 'start' as const, backOnlineAt: null },
            { at: at(4), kind: 'crash' as const, backOnlineAt: null },
        ];
        expect(crashPeriods(markers, FROM, at(6))).toEqual([
            { from: at(1), to: at(2) },
            { from: at(4), to: at(6) },
        ]);
    });
});
