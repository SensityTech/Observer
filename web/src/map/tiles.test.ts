import { describe, expect, it } from 'vitest';

import { toFraction } from './geometry';
import { TILES_BOUNDS, TILES_WORLD, isTileStyle, tileUrl, tilesIn, zoomFor } from './tiles';

/** Where a game position falls on the map at its base zoom, in pixels. */
const pixel = (x: number, y: number) => {
    const fraction = toFraction(TILES_BOUNDS, x, y);
    return { x: fraction.x * TILES_WORLD.width, y: fraction.y * TILES_WORLD.height };
};

describe('the tiles of Rockstar’s map', () => {
    it('should place the game on the map by the measured scale and offset', () => {
        // The game's origin, in Los Santos, and a point 1,000 units east and north of it.
        expect(pixel(0, 0).x).toBeCloseTo(941.13, 2);
        expect(pixel(0, 0).y).toBeCloseTo(1903.74, 2);
        expect(pixel(1000, 1000).x).toBeCloseTo(941.13 + 223.17, 2);
        expect(pixel(1000, 1000).y).toBeCloseTo(1903.74 - 226.32, 2);
        expect(TILES_BOUNDS.west).toBeLessThan(-4000);
        expect(TILES_BOUNDS.north).toBeGreaterThan(8000);
    });

    it('should name a tile as Rockstar’s servers do', () => {
        expect(tileUrl('atlas', 4, 3, 5)).toBe('https://s.rsg.sc/sc/images/games/GTAV/map/game/4/3/5.jpg');
        expect(tileUrl('satellite', 0, 0, 0)).toContain('/render/0/0/0.jpg');
        expect(tileUrl('roads', 7, 60, 90)).toContain('/print/7/60/90.jpg');
        expect(isTileStyle('atlas')).toBe(true);
        expect(isTileStyle('image')).toBe(false);
        expect(isTileStyle('grid')).toBe(false);
    });

    it('should pick a zoom as fine as the screen and no deeper than the servers hold', () => {
        expect(zoomFor(1, 1)).toBe(4);
        expect(zoomFor(1, 2)).toBe(5);
        expect(zoomFor(0.5, 1)).toBe(3);
        expect(zoomFor(0.05, 1)).toBe(0);
        expect(zoomFor(64, 2)).toBe(7);
        // A little past a zoom's own scale keeps it rather than quadrupling the tiles.
        expect(zoomFor(1.2, 1)).toBe(4);
    });

    it('should draw the tiles in view only, edge to edge', () => {
        // The whole map at its base zoom in a window as large: 8 by 12 tiles.
        const whole = tilesIn('atlas', { scale: 1, x: 0, y: 0 }, { width: 2048, height: 3072 }, 1);
        expect(whole.over).toHaveLength(96);
        expect(whole.under).toMatchObject({ left: 0, top: 0, width: 4096, height: 4096 });

        // A window of 800 by 600 on its middle, at a scale no tile edge falls on a whole pixel.
        const view = { scale: 1.37, x: -900.4, y: -1700.7 };
        const { over } = tilesIn('satellite', view, { width: 800, height: 600 }, 1);
        expect(over.length).toBeGreaterThan(0);
        expect(over.length).toBeLessThan(30);
        for (const tile of over) {
            expect(tile.left + tile.width).toBeGreaterThan(0);
            expect(tile.left).toBeLessThan(800);
            expect(tile.top + tile.height).toBeGreaterThan(0);
            expect(tile.top).toBeLessThan(600);
            const right = over.find((other) => other.top === tile.top && other.left > tile.left);
            if (right !== undefined) {
                expect(
                    over.some((other) => other.top === tile.top && other.left === tile.left + tile.width),
                ).toBe(true);
            }
        }
    });

    it('should draw nothing but the whole map when the view is off it, and never more tiles than a screen needs', () => {
        const away = tilesIn('roads', { scale: 1, x: 5000, y: 5000 }, { width: 800, height: 600 }, 1);
        expect(away.over).toHaveLength(0);

        const huge = tilesIn('atlas', { scale: 8, x: 0, y: 0 }, { width: 7680, height: 4320 }, 3);
        expect(huge.over.length).toBeLessThanOrEqual(240);
    });
});
