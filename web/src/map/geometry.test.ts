import { describe, expect, it } from 'vitest';

import { boundsFrom, niceStep, tierOf, toFraction } from './geometry';

const BOUNDS = { west: -5000, east: 5000, south: -4000, north: 8000 };

describe('geometry', () => {
    it('should place the game on the image north up, x to the east', () => {
        expect(toFraction(BOUNDS, -5000, 8000)).toEqual({ x: 0, y: 0 });
        expect(toFraction(BOUNDS, 5000, -4000)).toEqual({ x: 1, y: 1 });
        expect(toFraction(BOUNDS, 0, 2000)).toEqual({ x: 0.5, y: 0.5 });
    });

    it('should find the edges from two spots, and refuse spots too close together', () => {
        // On a 1000 × 1200 image of BOUNDS, ten game units a pixel.
        const first = { pixel: { x: 100, y: 200 }, world: { x: -4000, y: 6000 } };
        const second = { pixel: { x: 900, y: 1000 }, world: { x: 4000, y: -2000 } };

        expect(boundsFrom(first, second, 1000, 1200)).toEqual(BOUNDS);
        expect(boundsFrom(first, { ...second, pixel: { x: 120, y: 1000 } }, 1000, 1200)).toBeNull();
        expect(boundsFrom(first, { ...second, world: { x: -4010, y: -2000 } }, 1000, 1200)).toBeNull();
        expect(boundsFrom(first, { ...second, world: { x: 4000, y: 9000 } }, 1000, 1200)).toBeNull();
    });

    it('should colour a player by play time once confirmed', () => {
        const settings = { newHours: 10, veteranHours: 100 };

        expect(tierOf(null, settings)).toBe('unconfirmed');
        expect(tierOf(9.9 * 3_600_000, settings)).toBe('new');
        expect(tierOf(10 * 3_600_000, settings)).toBe('regular');
        expect(tierOf(100 * 3_600_000, settings)).toBe('veteran');
    });

    it('should round a grid step to 1, 2 or 5 of a power of ten', () => {
        expect(niceStep(420)).toBe(500);
        expect(niceStep(1500)).toBe(2000);
        expect(niceStep(90)).toBe(100);
        expect(niceStep(1)).toBe(1);
    });
});
