import { describe, expect, it } from 'vitest';

import { TRAIL_MS, extend, type Trails } from './trails';

const at = (netId: number, x: number, y: number) => ({ netId, x, y });
const places = (trails: Trails, netId: number) =>
    (trails.get(netId) ?? []).map((point) => [point.x, point.y]);

describe('extend', () => {
    it('should add a place for each player who moved, and none for one who stands still', () => {
        let trails: Trails = new Map();
        trails = extend(trails, [at(1, 0, 0), at(2, 100, 100)], 1000);
        trails = extend(trails, [at(1, 10, 0), at(2, 100.5, 100)], 3000);
        trails = extend(trails, [at(1, 20, 5), at(2, 100.5, 101)], 5000);

        expect(places(trails, 1)).toEqual([
            [0, 0],
            [10, 0],
            [20, 5],
        ]);
        expect(places(trails, 2)).toEqual([[100, 100]]);
    });

    it('should forget the places older than five minutes, and whoever left the map', () => {
        let trails: Trails = new Map();
        trails = extend(trails, [at(1, 0, 0), at(2, 50, 50)], 0);
        trails = extend(trails, [at(1, 10, 0), at(2, 60, 50)], 60_000);
        trails = extend(trails, [at(1, 20, 0)], TRAIL_MS + 1);

        expect(places(trails, 1)).toEqual([
            [10, 0],
            [20, 0],
        ]);
        expect(trails.has(2)).toBe(false);
    });

    it('should keep a trail to a bounded number of places', () => {
        let trails: Trails = new Map();
        for (let step = 0; step < 400; step += 1) {
            trails = extend(trails, [at(1, step * 5, 0)], step * 100);
        }

        expect(trails.get(1)).toHaveLength(160);
        expect(trails.get(1)?.at(-1)?.x).toBe(1995);
    });
});
