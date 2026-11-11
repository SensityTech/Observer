import { describe, expect, it } from 'vitest';

import type { EntityCell } from '../api/generated/EntityCell';
import type { EntityPoint } from '../api/generated/EntityPoint';
import { around, boundsFor, cellAt, cellBounds, cellFor, nearest, shortCount, strength } from './layer';

describe('cellFor', () => {
    it('should pick the side that is about 36 px on screen, among those the API takes', () => {
        expect(cellFor(0.1)).toBe(25);
        expect(cellFor(1)).toBe(50);
        expect(cellFor(5)).toBe(200);
        expect(cellFor(5.6)).toBe(500);
        expect(cellFor(40)).toBe(2000);
        expect(cellFor(500)).toBe(2000);
    });
});

describe('boundsFor', () => {
    const view = { west: 100, east: 1100, south: -1400, north: -800 };

    it('should hold the whole view, and no more than three times its span', () => {
        const asked = boundsFor(view);

        expect(asked).toEqual({ west: 0, east: 1536, south: -1536, north: -512 });
        expect(asked.east - asked.west).toBeLessThanOrEqual(3 * (view.east - view.west));
    });

    it('should stay the same while the map is dragged within a square', () => {
        const dragged = { west: 140, east: 1140, south: -1430, north: -830 };

        expect(boundsFor(dragged)).toEqual(boundsFor(view));
    });

    it('should ask for less once the map is zoomed in', () => {
        const closer = { west: 500, east: 700, south: -1200, north: -1080 };

        expect(boundsFor(closer)).toEqual({ west: 384, east: 768, south: -1280, north: -1024 });
    });

    it('should ask for a part with a width even of a view without any', () => {
        const point = { west: 10, east: 10, south: 20, north: 20 };

        expect(boundsFor(point)).toEqual({ west: 10, east: 11, south: 20, north: 21 });
    });
});

describe('nearest', () => {
    const points: EntityPoint[] = [
        [12, 120, 2, 100, 100],
        [13, 130, 1, 104, 100],
        [14, 140, 3, 300, 300],
    ];

    it('should pick the entity closest to the press, within its reach', () => {
        expect(nearest(points, { x: 103, y: 100, reach: 5 })?.[0]).toBe(13);
        expect(nearest(points, { x: 99, y: 101, reach: 5 })?.[0]).toBe(12);
    });

    it('should pick nothing when the press reaches no entity', () => {
        expect(nearest(points, { x: 200, y: 200, reach: 5 })).toBeNull();
        expect(nearest([], { x: 0, y: 0, reach: 50 })).toBeNull();
    });
});

describe('cells', () => {
    const cells: EntityCell[] = [
        [0, -5, 84, 3],
        [-1, -5, 12, 2],
    ];

    it('should find the cell a place falls in, west of the origin too', () => {
        expect(cellAt(cells, 200, 150, -850)).toEqual([0, -5, 84, 3]);
        expect(cellAt(cells, 200, -1, -801)).toEqual([-1, -5, 12, 2]);
        expect(cellAt(cells, 200, 450, -850)).toBeNull();
    });

    it('should give the part of the game a cell covers', () => {
        expect(cellBounds([-1, -5, 12, 2], 200)).toEqual({ west: -200, east: 0, south: -1000, north: -800 });
    });

    it('should draw the fullest cell the strongest, and a nearly empty one still visibly', () => {
        expect(strength(2100, 2100)).toBe(1);
        expect(strength(1, 2100)).toBeGreaterThan(0.05);
        expect(strength(84, 2100)).toBeLessThan(strength(310, 2100));
        expect(strength(1, 1)).toBe(1);
    });

    it('should write a count as a cell has room for', () => {
        expect(shortCount(84)).toBe('84');
        expect(shortCount(999)).toBe('999');
        expect(shortCount(1000)).toBe('1k');
        expect(shortCount(1240)).toBe('1.2k');
        expect(shortCount(12_400)).toBe('12k');
    });
});

describe('around', () => {
    it('should frame a place with some of the map around it', () => {
        expect(around(412.6, -1208.3, 60)).toEqual({
            west: 352.6,
            east: 472.6,
            south: -1268.3,
            north: -1148.3,
        });
    });
});
