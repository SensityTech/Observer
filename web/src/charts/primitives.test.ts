import type { ITimeScaleApi, Time } from 'lightweight-charts';
import { describe, expect, it } from 'vitest';

import { instantX } from './primitives';

const HOUR = 3_600;

/** A time scale whose points are `starts`, 100 pixels apart, and nothing between them. */
const scale = (starts: readonly number[]) =>
    ({
        timeToCoordinate: (time: Time) => {
            const index = starts.indexOf(time as number);
            return index < 0 ? null : index * 100;
        },
    }) as unknown as ITimeScaleApi<Time>;

describe('instantX', () => {
    it('should place an instant between its bucket start and the next one', () => {
        const starts = [0, HOUR, 2 * HOUR];
        expect(instantX(scale(starts), starts, HOUR, HOUR + 1_800)).toBe(150);
        expect(instantX(scale(starts), starts, HOUR, 0)).toBe(0);
    });

    it('should spread a day of 25 hours over the same width as the others', () => {
        // The middle day is 25 hours long: half of it is 12.5 hours in.
        const starts = [0, 24 * HOUR, 49 * HOUR, 73 * HOUR];
        expect(instantX(scale(starts), starts, 24 * HOUR, 24 * HOUR + 12.5 * HOUR)).toBe(150);
    });

    it('should give the last bucket the width of the one before, and nothing before the first', () => {
        const starts = [0, HOUR, 2 * HOUR];
        expect(instantX(scale(starts), starts, HOUR, 2 * HOUR + 900)).toBe(225);
        expect(instantX(scale(starts), starts, HOUR, 5 * HOUR)).toBe(300);
        expect(instantX(scale(starts), starts, HOUR, -1)).toBeNull();
    });
});
