// What the players routes answer, for the tests: a small community in Paris on 30 October 2026, a
// regular whose time partly came from txAdmin, and two players still being checked.

import type { Community } from '../api/generated/Community';
import type { Me } from '../api/generated/Me';
import type { OnlinePlayers } from '../api/generated/OnlinePlayers';
import type { PlayerPage } from '../api/generated/PlayerPage';
import type { PlayerProfile } from '../api/generated/PlayerProfile';
import type { PlayersSettings } from '../api/generated/PlayersSettings';
import { dayBefore } from '../api/players';

const HOUR = 3_600_000;

export const LICENSE = 'license:5f3c9a1be27d40c8a6f1d2e3b4c5d6e7f8091a2b';

export const OWNER: Me = {
    id: 1,
    username: 'ada',
    isOwner: true,
    permissions: '7fff',
    mustChangePassword: false,
};
/** A moderator who sees players, not their other identifiers. */
export const MODERATOR: Me = {
    id: 2,
    username: 'kenji',
    isOwner: false,
    permissions: '2000',
    mustChangePassword: false,
};

export const ONLINE: OnlinePlayers = {
    players: [
        {
            netId: 3,
            confirmation: 'confirmed',
            player: 7,
            name: 'Ruby Vex',
            license: LICENSE,
            since: '2026-10-30T18:02:00Z',
            sessionMs: 2 * HOUR + 14 * 60_000,
            totalMs: 314 * HOUR,
        },
        {
            netId: 12,
            confirmation: 'confirming',
            player: null,
            name: null,
            license: null,
            since: '2026-10-30T20:15:55Z',
            sessionMs: 5_000,
            totalMs: null,
        },
        {
            netId: 14,
            confirmation: 'unconfirmed',
            player: null,
            name: null,
            license: null,
            since: '2026-10-30T19:40:00Z',
            sessionMs: 36 * 60_000,
            totalMs: null,
        },
    ],
};

export const COMMUNITY: Community = {
    zone: 'Europe/Paris',
    today: '2026-10-30',
    days: Array.from({ length: 30 }, (_, index) => ({
        day: dayBefore('2026-10-30', 29 - index),
        players: 20 + (index % 7),
        newPlayers: index % 3,
        playMs: (40 + index) * HOUR,
    })),
    week: { players: 61, newPlayers: 7, playMs: 380 * HOUR },
    previousWeek: { players: 58, newPlayers: 9, playMs: 352 * HOUR },
};

export const PLAYERS: PlayerPage = {
    players: [
        {
            id: 7,
            name: 'Ruby Vex',
            license: LICENSE,
            playMs: 34 * HOUR,
            importedMs: 280 * HOUR,
            recentMs: 21 * HOUR,
            inProgressMs: 2 * HOUR + 14 * 60_000,
            sessions: 19,
            firstSeen: '2024-03-02T19:20:00Z',
            lastSeen: '2026-10-30T18:02:00Z',
            online: true,
        },
    ],
    next: null,
    zone: 'Europe/Paris',
};

export const PROFILE: PlayerProfile = {
    id: 7,
    name: 'Ruby Vex',
    license: LICENSE,
    firstSeen: '2024-03-02T19:20:00Z',
    lastSeen: '2026-10-30T18:02:00Z',
    importedAt: '2026-10-20T09:00:00Z',
    online: true,
    playMs: 34 * HOUR,
    importedMs: 280 * HOUR,
    inProgressMs: 2 * HOUR + 14 * 60_000,
    last7Ms: 9 * HOUR,
    last30Ms: 21 * HOUR,
    sessions: 19,
    longestMs: 5 * HOUR,
    crashes: 3,
    timeouts: 1,
    kicks: 0,
    crashRate: 8.8,
    serverCrashRate: 2.1,
    names: [{ name: 'Ruby Vex', firstSeen: '2026-10-21T18:00:00Z', lastSeen: '2026-10-30T18:02:00Z' }],
    days: [
        { day: '2026-10-29', playMs: 3 * HOUR },
        { day: '2026-10-30', playMs: 2 * HOUR + 14 * 60_000 },
    ],
    drops: [
        {
            endedAt: '2026-10-29T21:40:00Z',
            category: 'crash',
            kind: null,
            resource: null,
            lengthMs: 3 * HOUR,
        },
        {
            endedAt: '2026-10-27T20:00:00Z',
            category: 'kick',
            kind: 'resource',
            resource: 'afk_kick',
            lengthMs: HOUR,
        },
    ],
    identifiers: null,
    linked: null,
    zone: 'Europe/Paris',
    today: '2026-10-30',
};

export const SETTINGS: PlayersSettings = { linkDays: 365, addressDays: 90, dayDays: 400, profileDays: 730 };
