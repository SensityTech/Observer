import { describe, expect, it } from 'vitest';

import type { Community } from './generated/Community';
import type { PlayersImport } from './generated/PlayersImport';
import {
    dayBefore,
    fillDays,
    importQuery,
    playersSearch,
    retentionIssues,
    shortLicence,
    shortToken,
    weeklyPoints,
} from './players';

const DEFAULTS = { linkDays: 365, addressDays: 90, dayDays: 400, profileDays: 730 };

describe('playersSearch', () => {
    it('should leave out an empty search and the default sort', () => {
        expect(playersSearch('', 'playTime')).toBe('');
        expect(playersSearch('discord:123456789012345678', 'lastSeen', '1714.42')).toBe(
            '?search=discord%3A123456789012345678&sort=lastSeen&after=1714.42',
        );
    });
});

describe('retentionIssues', () => {
    it('should accept the defaults and a profile kept for good', () => {
        expect(retentionIssues(DEFAULTS)).toEqual([]);
        expect(retentionIssues({ ...DEFAULTS, linkDays: 5_000, dayDays: 5_000, profileDays: null })).toEqual(
            [],
        );
    });

    it('should refuse a retention that outlives what it hangs on, or a part of a day', () => {
        expect(retentionIssues({ ...DEFAULTS, profileDays: 364 })).toEqual([
            'profileDays',
            'linkDays',
            'dayDays',
        ]);
        expect(retentionIssues({ ...DEFAULTS, linkDays: 29 })).toEqual(['linkDays', 'addressDays']);
        expect(retentionIssues({ ...DEFAULTS, addressDays: 366 })).toEqual(['addressDays']);
        expect(retentionIssues({ ...DEFAULTS, dayDays: 30.5 })).toEqual(['dayDays']);
    });
});

describe('identifiers', () => {
    it('should shorten a licence to its first digits and a token after its type', () => {
        expect(shortLicence('license:1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d')).toBe('1a2b3c4d…');
        expect(shortToken(`3:${'ab'.repeat(32)}`)).toBe('3:abababababab…');
        expect(shortToken('3:abcd')).toBe('3:abcd');
    });
});

describe('days', () => {
    it('should count days back across months and leap days, as dates', () => {
        expect(dayBefore('2026-03-01', 1)).toBe('2026-02-28');
        expect(dayBefore('2028-03-01', 1)).toBe('2028-02-29');
        expect(dayBefore('2026-10-31', 30)).toBe('2026-10-01');
    });

    it('should fill the days without play with zero, up to today', () => {
        expect(fillDays([{ day: '2026-10-30', playMs: 600_000 }], '2026-10-31', 3)).toEqual([
            { day: '2026-10-29', playMs: 0 },
            { day: '2026-10-30', playMs: 600_000 },
            { day: '2026-10-31', playMs: 0 },
        ]);
    });
});

describe('weeklyPoints', () => {
    it('should add up weeks ending today and leave the unique players unknown past the last two', () => {
        const community: Community = {
            zone: 'Europe/Paris',
            today: '2026-10-30',
            days: Array.from({ length: 30 }, (_, index) => ({
                day: dayBefore('2026-10-30', 29 - index),
                players: 10,
                newPlayers: index % 2,
                playMs: 3_600_000,
            })),
            week: { players: 41, newPlayers: 3, playMs: 7 * 3_600_000 },
            previousWeek: { players: 38, newPlayers: 4, playMs: 7 * 3_600_000 },
        };

        const weeks = weeklyPoints(community);

        expect(weeks.map((week) => week.day)).toEqual([
            '2026-10-03',
            '2026-10-10',
            '2026-10-17',
            '2026-10-24',
        ]);
        expect(weeks.map((week) => week.players)).toEqual([null, null, 38, 41]);
        expect(weeks.map((week) => week.newPlayers)).toEqual([3, 4, 3, 4]);
        expect(weeks.every((week) => week.playMs === 7 * 3_600_000)).toBe(true);
        expect(weeks.map((week) => week.today)).toEqual([false, false, false, true]);
    });
});

/** An import as the API reports it, in `state`. */
const job = (state: PlayersImport['state']): PlayersImport => ({
    state,
    startedAt: '2026-10-30T20:00:00Z',
    finishedAt: state === 'writing' ? null : '2026-10-30T20:01:00Z',
    bytes: 1_024,
    read: 10,
    written: 4,
    imported: 0,
    updated: 0,
    forgotten: 0,
    invalid: 0,
});

describe('importQuery', () => {
    it('should read the import again every 2 s while it writes, and stop once it ended', () => {
        const every = importQuery.refetchInterval as (query: {
            state: { data: PlayersImport | null | undefined };
        }) => number | false;

        expect(every({ state: { data: job('writing') } })).toBe(2_000);
        expect(every({ state: { data: job('finished') } })).toBe(false);
        expect(every({ state: { data: job('failed') } })).toBe(false);
        expect(every({ state: { data: null } })).toBe(false);
    });
});
