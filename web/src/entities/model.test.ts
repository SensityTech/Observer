import { describe, expect, it } from 'vitest';

import type { EntityItem } from '../api/generated/EntityItem';
import { blocker, breakdown, isStale, keyOf, modelHash, outcomes, scripted } from './model';

const item = (changes: Partial<EntityItem>): EntityItem => ({
    netId: 1,
    serial: 11,
    kind: 'vehicle',
    class: 'car',
    model: 0xb779a091,
    x: 0,
    y: 0,
    z: 0,
    owned: false,
    owner: null,
    ownerName: null,
    bucket: 0,
    population: 5,
    resource: null,
    resourcePending: false,
    occupied: false,
    ...changes,
});

describe('modelHash', () => {
    it("should write a hash as the game's tools do", () => {
        expect(modelHash(0xb779a091)).toBe('0xB779A091');
        expect(modelHash(7)).toBe('0x00000007');
        expect(modelHash(0)).toBe('0x00000000');
    });
});

describe('keyOf', () => {
    it('should tell two entities that took the same net id apart', () => {
        expect(keyOf(item({ netId: 12, serial: 3 }))).not.toBe(keyOf(item({ netId: 12, serial: 4 })));
    });
});

describe('blocker', () => {
    it('should never offer a vehicle a player sits in, nor a train', () => {
        expect(blocker(item({ occupied: true }))).toBe('occupied');
        expect(blocker(item({ class: 'train' }))).toBe('train');
        expect(blocker(item({}))).toBeNull();
        expect(blocker(item({ kind: 'object', class: null }))).toBeNull();
    });
});

describe('scripted', () => {
    it("should take a resource's entity and a client script's for a script's", () => {
        expect(scripted(item({ resource: 'qb-garages' }))).toBe(true);
        expect(scripted(item({ population: 7 }))).toBe(true);
        expect(scripted(item({}))).toBe(false);
    });
});

describe('isStale', () => {
    const at = '2026-11-11T20:00:00Z';
    const after = (ms: number) => Date.parse(at) + ms;

    it("should call a list out of date three sweeps after the last one, on the server's cadence", () => {
        expect(isStale({ at, everyMs: 15_000 }, after(45_000), 3)).toBe(false);
        expect(isStale({ at, everyMs: 15_000 }, after(45_001), 3)).toBe(true);
        expect(isStale({ at, everyMs: 60_000 }, after(120_000), 3)).toBe(false);
    });

    it('should not call out of date a list that never came', () => {
        expect(isStale({ at: null, everyMs: 15_000 }, after(1_000_000), 3)).toBe(false);
    });
});

describe('breakdown', () => {
    it('should name the largest groups and count the rest', () => {
        const items = [
            ...Array.from({ length: 4 }, (_, index) =>
                item({ netId: index, resource: 'qb-garages', owned: true }),
            ),
            item({ netId: 10, model: 1 }),
            item({ netId: 11, model: 2 }),
            item({ netId: 12, model: 3, population: 7 }),
            item({ netId: 13, model: 4, kind: 'object', class: null }),
            item({ netId: 14, model: 5 }),
        ];

        const made = breakdown(items);

        expect(made.groups[0]).toEqual({
            model: 0xb779a091,
            kind: 'vehicle',
            resource: 'qb-garages',
            count: 4,
        });
        expect(made.groups.map((group) => group.model)).toEqual([0xb779a091, 1, 2, 3, 4]);
        expect(made.more).toBe(1);
        expect(made.scripted).toBe(5);
        expect(made.owned).toBe(4);
    });
});

describe('outcomes', () => {
    it('should count what was deleted and what was not, by what became of it', () => {
        const summary = outcomes([
            { netId: 1, outcome: 'deleted' },
            { netId: 2, outcome: 'occupied' },
            { netId: 3, outcome: 'gone' },
            { netId: 4, outcome: 'occupied' },
            { netId: 5, outcome: 'deleted' },
        ]);

        expect(summary).toEqual({
            asked: 5,
            deleted: 2,
            others: [
                ['occupied', 2],
                ['gone', 1],
            ],
        });
    });
});
