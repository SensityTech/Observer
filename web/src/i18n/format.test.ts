import { describe, expect, it } from 'vitest';

import {
    formatBytes,
    formatClock,
    formatDay,
    formatDuration,
    formatNumber,
    formatTimestamp,
    knownZone,
    plural,
    zoneOffset,
} from './format';

describe('schedule times', () => {
    const paris = 'Europe/Paris';

    it('should read a restart in the schedule zone across both clock changes', () => {
        expect(formatClock('en', '2026-03-29T01:30:00Z', paris)).toBe('03:30');
        expect(formatClock('en', '2026-10-25T00:30:00Z', paris)).toBe('02:30');
        expect(formatClock('en', '2026-10-25T01:30:00Z', paris)).toBe('02:30');
        expect(zoneOffset('en', paris, '2026-10-25T00:30:00Z')).toBe('UTC+2');
        expect(zoneOffset('en', paris, '2026-10-25T01:30:00Z')).toBe('UTC+1');
    });

    it('should name the day in the zone, and the offsets Intl calls GMT as UTC', () => {
        expect(formatDay('en', '2026-10-28T22:30:00Z', 'Asia/Kolkata')).toBe('Thu 29 Oct');
        expect(formatDay('en', '2026-10-28T22:30:00Z', paris, { weekday: false })).toBe('28 Oct');
        expect(zoneOffset('en', 'Asia/Kolkata', '2026-10-28T22:30:00Z')).toBe('UTC+5:30');
        expect(zoneOffset('en', 'UTC', '2026-10-28T22:30:00Z')).toBe('UTC');
    });

    it('should fall back to UTC for a zone the browser does not know', () => {
        expect(knownZone('Europe/Atlantis')).toBe('UTC');
        expect(formatClock('en', '2026-10-28T22:30:00Z', 'Europe/Atlantis')).toBe('22:30');
    });
});

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
