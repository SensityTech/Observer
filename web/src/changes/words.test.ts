import { describe, expect, it } from 'vitest';

import type { ChangeCounts } from '../api/generated/ChangeCounts';
import { en } from '../i18n/en';
import { RESTART } from './fixtures';
import { countsWords } from './words';

const NOTHING: ChangeCounts = {
    added: 0,
    removed: 0,
    updated: 0,
    sameVersion: 0,
    renamed: 0,
    moved: 0,
    didNotStart: 0,
    checking: 0,
    unchanged: 125,
    files: 0,
    environment: [],
};

describe('countsWords', () => {
    it('should say why a start has no counts rather than call it listed for ever', () => {
        expect(countsWords(en, { ...RESTART, state: 'listing', counts: null })).toBe(en.changes.beingListed);
        expect(countsWords(en, { ...RESTART, state: 'failed', counts: null })).toBe(
            en.changes.snapshotFailed,
        );
        expect(countsWords(en, { ...RESTART, state: 'complete', counts: null })).toBe(en.changes.notCompared);
    });

    it('should never say no change while resources are still being checked, and say how many', () => {
        expect(countsWords(en, { ...RESTART, counts: NOTHING })).toBe(en.changes.noChange);
        expect(countsWords(en, { ...RESTART, state: 'hashing', counts: { ...NOTHING, checking: 4 } })).toBe(
            '4 resources being checked',
        );
        expect(
            countsWords(en, {
                ...RESTART,
                state: 'partial',
                counts: { ...NOTHING, updated: 1, checking: 1 },
            }),
        ).toBe('1 resource changed · 1 resource unknown');
    });
});
