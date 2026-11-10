import { describe, expect, it } from 'vitest';

import {
    formatBitRate,
    formatBytes,
    formatCalendarDay,
    formatClock,
    formatDay,
    formatDuration,
    formatElapsed,
    formatMilliseconds,
    formatNumber,
    formatPercent,
    formatPlayTime,
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

    it('should pass to kilobytes a rate that rounds to 1024 bytes', () => {
        expect(formatBytes('en', 1023.4)).toBe('1,023 bytes');
        expect(formatBytes('en', 1023.6)).toBe('1 KB');
    });

    it('should sign a growth, but not one that rounds to nothing', () => {
        expect(formatBytes('en', 180 * 1024 ** 2, { signed: true })).toBe('+180 MB');
        expect(formatBytes('en', -2048, { signed: true })).toBe('-2 KB');
        expect(formatBytes('en', 0.3, { signed: true })).toBe('0 bytes');
        expect(formatBytes('en', -0.3, { signed: true })).toBe('0 bytes');
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

    it('should keep two significant digits under a millisecond rather than round to zero', () => {
        expect(formatMilliseconds('en', 0.042)).toBe('0.042 ms');
        expect(formatMilliseconds('en', 0.0349)).toBe('0.035 ms');
        expect(formatMilliseconds('en', 0.826)).toBe('0.83 ms');
        expect(formatMilliseconds('en', 0)).toBe('0 ms');
    });
});

describe('formatNumber', () => {
    it('should group thousands for the locale', () => {
        expect(formatNumber('en', 1284)).toBe('1,284');
    });
});

describe('formatElapsed', () => {
    it('should count a running time to the second under an hour, never ahead of it', () => {
        expect(formatElapsed('en', 0.9)).toBe('0 s');
        expect(formatElapsed('en', 59.9)).toBe('59 s');
        expect(formatElapsed('en', 152)).toBe('2 min 32 s');
        expect(formatElapsed('en', 360)).toBe('6 min');
        expect(formatElapsed('en', 3599)).toBe('59 min 59 s');
        expect(formatElapsed('en', 2 * 3600 + 14 * 60)).toBe('2h 14m');
        expect(formatElapsed('en', -5)).toBe('0 s');
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

describe('formatPlayTime', () => {
    it('should count play time in hours past the first, never in days', () => {
        expect(formatPlayTime('en', 0)).toBe('0h');
        expect(formatPlayTime('en', 999)).toBe('0h');
        expect(formatPlayTime('en', 45_900)).toBe('45 s');
        expect(formatPlayTime('en', 15 * 60_000 + 59_000)).toBe('15 min');
        expect(formatPlayTime('en', (2 * 60 + 14) * 60_000)).toBe('2h 14m');
        expect(formatPlayTime('en', 3 * 3_600_000)).toBe('3h');
        expect(formatPlayTime('en', 50 * 3_600_000 + 5 * 60_000)).toBe('50h 5m');
        expect(formatPlayTime('en', 312 * 3_600_000 + 47 * 60_000)).toBe('312h');
        expect(formatPlayTime('en', 1_284 * 3_600_000)).toBe('1,284h');
        expect(formatPlayTime('en', -3_600_000)).toBe('0h');
    });

    it('should sign a difference of play time as Intl signs numbers', () => {
        expect(formatPlayTime('en', (2 * 60 + 14) * 60_000, { signed: true })).toBe('+2h 14m');
        expect(formatPlayTime('en', -62 * 3_600_000, { signed: true })).toBe(
            formatNumber('en', -62, { signDisplay: 'exceptZero' }) + 'h',
        );
        expect(formatPlayTime('en', 0, { signed: true })).toBe('0h');
    });
});

describe('formatCalendarDay', () => {
    it('should name a day of the API as it is written, whatever the zone of the browser', () => {
        expect(formatCalendarDay('en', '2026-10-28')).toBe('Wed 28 Oct');
        expect(formatCalendarDay('en', '2026-03-01', { weekday: false })).toBe('1 Mar');
    });
});
