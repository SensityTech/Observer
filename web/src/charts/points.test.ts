import { describe, expect, it } from 'vitest';

import { playerPoints, resumes, type Bucket } from './points';

const row = (time: number, players: number | null): Bucket => ({ time, players, slots: 64 });

describe('playerPoints', () => {
    it('should draw the point before a stop transparent, and the stop as whitespace', () => {
        const points = playerPoints([row(0, 37), row(5, null), row(10, 37)], 'clear');

        expect(points).toEqual([
            { time: 0, value: 37, lineColor: 'clear', topColor: 'clear', bottomColor: 'clear' },
            { time: 5 },
            { time: 10, value: 37 },
        ]);
    });

    it('should keep the newest point opaque while what follows is still to come', () => {
        expect(playerPoints([row(0, 37), row(5, null)], 'clear')[0]).toEqual({ time: 0, value: 37 });
    });
});

describe('resumes', () => {
    it('should tell when the newest bucket picks the line up after a stop', () => {
        expect(resumes([row(0, 37), row(5, null), row(10, 37)])).toBe(true);
        expect(resumes([row(0, 37), row(5, 37), row(10, 37)])).toBe(false);
        expect(resumes([row(0, 37), row(5, 37), row(10, null)])).toBe(false);
        expect(resumes([row(0, 37)])).toBe(false);
    });
});
