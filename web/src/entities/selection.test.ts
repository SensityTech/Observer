import { describe, expect, it } from 'vitest';

import type { EntityItem } from '../api/generated/EntityItem';
import { NOTHING, pageState, pageToggled, refreshed, targetsOf, toggled } from './selection';

const item = (netId: number, changes: Partial<EntityItem> = {}): EntityItem => ({
    netId,
    serial: netId * 10,
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

const page = [item(1), item(2, { occupied: true }), item(3, { class: 'train' }), item(4)];

describe('toggled', () => {
    it('should pick an entity, and let it go when asked again', () => {
        const picked = toggled(NOTHING, item(1), 100);

        expect([...picked.keys()]).toEqual(['1:10']);
        expect(toggled(picked, item(1), 100).size).toBe(0);
    });

    it('should never pick a vehicle a player sits in, nor a train', () => {
        expect(toggled(NOTHING, item(2, { occupied: true }), 100)).toBe(NOTHING);
        expect(toggled(NOTHING, item(3, { class: 'train' }), 100)).toBe(NOTHING);
    });

    it('should stop at the most a deletion takes, and still let go', () => {
        const full = toggled(toggled(NOTHING, item(1), 2), item(4), 2);

        expect(toggled(full, item(5), 2)).toBe(full);
        expect(toggled(full, item(4), 2).size).toBe(1);
    });

    it('should tell two entities that took the same net id apart', () => {
        const picked = toggled(NOTHING, item(1), 100);

        expect(toggled(picked, item(1, { serial: 77 }), 100).size).toBe(2);
    });
});

describe('pageToggled', () => {
    it('should pick what the page offers, and let the page go when it all was picked', () => {
        const picked = pageToggled(NOTHING, page, 100);

        expect([...picked.keys()]).toEqual(['1:10', '4:40']);
        expect(pageState(picked, page)).toBe('all');
        expect(pageToggled(picked, page, 100).size).toBe(0);
    });

    it('should keep what other pages picked, and fill up to the most a deletion takes', () => {
        const elsewhere = toggled(NOTHING, item(9), 2);
        const picked = pageToggled(elsewhere, page, 2);

        expect([...picked.keys()]).toEqual(['9:90', '1:10']);
        expect(pageState(picked, page)).toBe('some');
        expect(pageState(elsewhere, page)).toBe('none');
    });
});

describe('refreshed', () => {
    it('should let go an entity a player got into, and keep the others as they now are', () => {
        const picked = pageToggled(NOTHING, page, 100);
        const moved = item(4, { x: 12 });

        const now = refreshed(picked, [item(1, { occupied: true }), moved]);

        expect([...now.values()]).toEqual([moved]);
    });

    it('should give the same selection back when nothing of it changed', () => {
        const picked = pageToggled(NOTHING, page, 100);

        expect(refreshed(picked, page)).toBe(picked);
        expect(refreshed(picked, [item(7)])).toBe(picked);
    });
});

describe('targetsOf', () => {
    it('should name each entity by its net id, its serial and its model', () => {
        expect(targetsOf(pageToggled(NOTHING, page, 100))).toEqual([
            { netId: 1, serial: 10, model: 0xb779a091 },
            { netId: 4, serial: 40, model: 0xb779a091 },
        ]);
    });
});
