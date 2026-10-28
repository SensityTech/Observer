import { describe, expect, it } from 'vitest';

import {
    formatBitRate,
    formatBytes,
    formatDuration,
    formatMilliseconds,
    formatNumber,
    formatPercent,
    formatTimestamp,
    plural,
} from './format';

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
    it('should count in multiples of 1024 with three significant digits', () => {
        expect(formatBytes('en', 9.4 * 1024 ** 3)).toBe('9.4 GB');
        expect(formatBytes('en', 38 * 1024 ** 2)).toBe('38 MB');
        expect(formatBytes('en', 22.14 * 1024 ** 3)).toBe('22.1 GB');
        expect(formatBytes('en', 64 * 1024 ** 3)).toBe('64 GB');
        expect(formatBytes('en', 412.4 * 1024)).toBe('412 KB');
        expect(formatBytes('en', 3 * 1024 ** 4)).toBe('3 TB');
    });

    it('should spell out whole bytes below a kilobyte, plural included', () => {
        expect(formatBytes('en', 512)).toBe('512 bytes');
        expect(formatBytes('en', 1)).toBe('1 byte');
    });
});

describe('formatBitRate', () => {
    it('should give bytes per second as bits in multiples of 1000', () => {
        expect(formatBitRate('en', 1_500_000)).toBe('12 Mbit/s');
        expect(formatBitRate('en', 2000)).toBe('16 kbit/s');
        expect(formatBitRate('en', 50)).toBe('400 bit/s');
        expect(formatBitRate('en', 1.25e9)).toBe('10 Gbit/s');
    });
});

describe('formatPercent', () => {
    it('should keep the sign against the number with one decimal at most', () => {
        expect(formatPercent('en', 0.42)).toBe('0.4%');
        expect(formatPercent('en', 71)).toBe('71%');
        expect(formatPercent('en', 182.4)).toBe('182%');
    });
});

describe('formatMilliseconds', () => {
    it('should write three significant digits', () => {
        expect(formatMilliseconds('en', 6.84)).toBe('6.8 ms');
        expect(formatMilliseconds('en', 412.3)).toBe('412 ms');
        expect(formatMilliseconds('en', 1250)).toBe('1,250 ms');
    });
});

describe('formatNumber', () => {
    it('should group thousands for the locale', () => {
        expect(formatNumber('en', 1284)).toBe('1,284');
    });
});

describe('formatDuration', () => {
    it('should count seconds under a minute and whole minutes, rounded up, under an hour', () => {
        expect(formatDuration('en', 12)).toBe('12 s');
        expect(formatDuration('en', 0.2)).toBe('1 s');
        expect(formatDuration('en', 60)).toBe('1 min');
        expect(formatDuration('en', 899)).toBe('15 min');
    });

    it('should write hours and minutes, dropping zero minutes', () => {
        expect(formatDuration('en', 2 * 3600 + 14 * 60)).toBe('2h 14m');
        expect(formatDuration('en', 3 * 3600)).toBe('3h');
    });

    it('should write days and hours beyond a day', () => {
        expect(formatDuration('en', 3 * 86_400 + 4 * 3600)).toBe('3d 4h');
        expect(formatDuration('en', 7 * 86_400)).toBe('7d');
    });

    it('should never show a negative wait', () => {
        expect(formatDuration('en', -5)).toBe('0 s');
    });
});

describe('formatTimestamp', () => {
    const moment = new Date(2026, 9, 28, 14, 2, 7);

    it('should write local dates in ISO order, seconds included by default', () => {
        expect(formatTimestamp(moment)).toBe('2026-10-28 14:02:07');
    });

    it('should drop the seconds on request', () => {
        expect(formatTimestamp(moment, { seconds: false })).toBe('2026-10-28 14:02');
    });
});
