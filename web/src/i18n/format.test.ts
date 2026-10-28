import { describe, expect, it } from 'vitest';

import { formatBytes, formatNumber, plural } from './format';

describe('plural', () => {
    const players = { one: 'player', other: 'players' };

    it('should pick the English singular for one only', () => {
        expect(plural('en', 1, players)).toBe('player');
        expect(plural('en', 0, players)).toBe('players');
        expect(plural('en', 2, players)).toBe('players');
    });

    it('should treat zero as singular in French', () => {
        expect(plural('fr', 0, { one: 'joueur', other: 'joueurs' })).toBe('joueur');
    });

    it('should fall back to the other form when a category is missing', () => {
        expect(plural('en', 1, { other: 'players' })).toBe('players');
    });
});

describe('formatBytes', () => {
    it('should use decimal units with one decimal', () => {
        expect(formatBytes('en', 9_400_000_000)).toBe('9.4 GB');
        expect(formatBytes('en', 38_000_000)).toBe('38 MB');
    });

    it('should spell out whole bytes below a kilobyte, plural included', () => {
        expect(formatBytes('en', 512)).toBe('512 bytes');
        expect(formatBytes('en', 1)).toBe('1 byte');
    });
});

describe('formatNumber', () => {
    it('should group thousands for the locale', () => {
        expect(formatNumber('en', 1284)).toBe('1,284');
    });
});
