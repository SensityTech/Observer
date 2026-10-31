// What the drops routes answer, for the tests: a week of a server in Paris up to the evening of
// 30 October 2026, crashes rising after a build change, an AFK kicker, and one hitch that dropped
// a crowd. Statistics viewers get the same figures without names, reason groups, hidden signatures
// or examples a single player wrote.

import type { DropChanges } from '../api/generated/DropChanges';
import type { DropLog } from '../api/generated/DropLog';
import type { DropSeries } from '../api/generated/DropSeries';
import type { DropsSettings } from '../api/generated/DropsSettings';
import type { DropsSummary } from '../api/generated/DropsSummary';
import type { DropUptime } from '../api/generated/DropUptime';
import type { Me } from '../api/generated/Me';
import type { ResourceTable } from '../api/generated/ResourceTable';
import type { SignatureTable } from '../api/generated/SignatureTable';
import type { Trend } from '../api/generated/Trend';

const HOUR = 3_600_000;

export const LICENSE = 'license:5f3c9a1be27d40c8a6f1d2e3b4c5d6e7f8091a2b';

/** The owner holds every permission. */
export const OWNER: Me = {
    id: 1,
    username: 'ada',
    isOwner: true,
    permissions: '1ffff',
    mustChangePassword: false,
};
/** A moderator who sees players, and so their drops: players imply the statistics. */
export const MODERATOR: Me = {
    id: 2,
    username: 'kenji',
    isOwner: false,
    permissions: '14000',
    mustChangePassword: false,
};
/** An analyst who sees the statistics, never a player. */
export const ANALYST: Me = {
    id: 3,
    username: 'mira',
    isOwner: false,
    permissions: '10000',
    mustChangePassword: false,
};

export const RISING: Trend = {
    verdict: 'rising',
    change: 0.38,
    low: 0.12,
    high: 0.7,
    now: 4.6,
    before: 3.3,
    eventsNow: 64,
    eventsBefore: 41,
};

export const FALLING: Trend = {
    verdict: 'falling',
    change: -0.2,
    low: -0.31,
    high: -0.07,
    now: 2.4,
    before: 3,
    eventsNow: 52,
    eventsBefore: 61,
};

export const STEADY: Trend = {
    verdict: 'noClearChange',
    change: 0.08,
    low: -0.1,
    high: 0.29,
    now: 1.3,
    before: 1.2,
    eventsNow: 28,
    eventsBefore: 26,
};

export const FEW: Trend = {
    verdict: 'notEnoughDrops',
    change: 1,
    low: null,
    high: null,
    now: 0.4,
    before: 0.2,
    eventsNow: 4,
    eventsBefore: 2,
};

const spark = (base: number) =>
    Array.from({ length: 24 }, (_, index) => (index % 6 === 5 ? null : base + (index % 4)));

export const SUMMARY: DropsSummary = {
    range: '7d',
    zone: 'Europe/Paris',
    from: '2026-10-23T19:00:00Z',
    to: '2026-10-30T19:00:00Z',
    trendFrom: '2026-10-23T19:00:00Z',
    beforeFrom: '2026-10-16T19:00:00Z',
    beforeTo: '2026-10-23T19:00:00Z',
    tiles: [
        {
            kind: 'crashes',
            drops: 64,
            rate: 4.6,
            oneIn: 23,
            trend: RISING,
            spark: spark(4),
            onePlayerShare: 0.41,
        },
        {
            kind: 'timeouts',
            drops: 52,
            rate: 2.4,
            oneIn: 28,
            trend: FALLING,
            spark: spark(2),
            onePlayerShare: null,
        },
        {
            kind: 'kicks',
            drops: 28,
            rate: 1.3,
            oneIn: 52,
            trend: STEADY,
            spark: spark(1),
            onePlayerShare: null,
        },
        {
            kind: 'unexpected',
            drops: 131,
            rate: 9.4,
            oneIn: 11,
            trend: STEADY,
            spark: spark(9),
            onePlayerShare: null,
        },
    ],
    categories: [
        {
            category: 'quit',
            expected: true,
            drops: 1_280,
            share: 0.84,
            rate: 92,
            trend: STEADY,
            loadingShare: null,
        },
        {
            category: 'crash',
            expected: false,
            drops: 64,
            share: 0.042,
            rate: 4.6,
            trend: RISING,
            loadingShare: 0.25,
        },
        {
            category: 'timeout',
            expected: false,
            drops: 52,
            share: 0.034,
            rate: 2.4,
            trend: FALLING,
            loadingShare: 0.1,
        },
        {
            category: 'kick',
            expected: true,
            drops: 28,
            share: 0.018,
            rate: 1.3,
            trend: STEADY,
            loadingShare: null,
        },
        {
            category: 'security',
            expected: false,
            drops: 9,
            share: 0.006,
            rate: 0.4,
            trend: FEW,
            loadingShare: null,
        },
        {
            category: 'unknown',
            expected: false,
            drops: 6,
            share: 0.004,
            rate: 0.3,
            trend: FEW,
            loadingShare: null,
        },
        {
            category: 'serverStop',
            expected: true,
            drops: 70,
            share: 0.046,
            rate: null,
            trend: null,
            loadingShare: null,
        },
        {
            category: 'serverCrash',
            expected: false,
            drops: 18,
            share: 0.012,
            rate: null,
            trend: null,
            loadingShare: null,
        },
    ],
    playedMs: 1_390 * HOUR,
    sessions: 1_520,
    repeatCrashers: [{ crashes: 7, player: { id: 7, license: LICENSE, name: 'Ruby Vex' } }],
    coverage: { sessions: 1_520, missed: 70, lost: 12 },
    keptFrom: '2026-10-23T19:00:00Z',
};

/** The same week for a statistics viewer: the repeat crasher counted, not named. */
export const SUMMARY_STATS: DropsSummary = {
    ...SUMMARY,
    repeatCrashers: [{ crashes: 7, player: null }],
};

/** Today since midnight in Paris against yesterday's same hours. */
export const SUMMARY_DAY: DropsSummary = {
    ...SUMMARY,
    range: '24h',
    from: '2026-10-29T19:00:00Z',
    trendFrom: '2026-10-29T23:00:00Z',
    beforeFrom: '2026-10-28T23:00:00Z',
    beforeTo: '2026-10-29T19:00:00Z',
};

/** Six hours of crashes: nobody played in the third, a crash at 21:40, a build change at 22:00. */
export const SERIES: DropSeries = {
    range: '7d',
    category: 'crash',
    from: 1_761_854_400,
    step: 3_600,
    starts: Array.from({ length: 6 }, (_, index) => 1_761_854_400 + index * 3_600),
    rates: [3.1, 4.2, null, 5, 6.4, 4.4],
    baseline: [null, 3.2, 3.4, 3.3, 3.1, 3],
    drops: [2, 3, 0, 4, 6, 3],
    markers: [
        {
            at: 1_761_854_400 + 2 * 3_600 + 2_400,
            kind: 'crash',
            backOnlineAt: 1_761_854_400 + 2 * 3_600 + 2_520,
        },
    ],
    changes: [{ at: 1_761_854_400 + 3 * 3_600, build: 17000 }],
};

export const SIGNATURES: SignatureTable = {
    range: '7d',
    crashes: 64,
    signatures: [
        {
            id: 11,
            signature: 'load:c0000005:cars',
            form: 'load',
            label: null,
            module: null,
            gameBuild: 3095,
            resource: 'cars',
            drops: 31,
            share: 0.48,
            players: 12,
            firstSeen: '2026-10-29T22:10:00Z',
            new: true,
            loadingShare: 0.9,
            trend: RISING,
            examples: [
                'Recursive error: An exception occurred (c0000005 at 0x{n}) during loading of resources:/cars/data/<b>xmas</b>/carvariations.meta',
            ],
        },
        {
            id: 12,
            signature: 'hash:GTA5_b3095.exe+1A2B3C',
            form: 'hash',
            label: 'kilo-ohio-lemon',
            module: 'GTA5.exe',
            gameBuild: 3095,
            resource: null,
            drops: 18,
            share: 0.28,
            players: 9,
            firstSeen: '2026-09-02T18:00:00Z',
            new: false,
            loadingShare: 0.05,
            trend: STEADY,
            examples: ['kilo-ohio-lemon (GTA5_b3095.exe+1A2B3C)'],
        },
        {
            id: 13,
            signature: null,
            form: 'text',
            label: null,
            module: null,
            gameBuild: null,
            resource: null,
            drops: 2,
            share: 0.03,
            players: 1,
            firstSeen: '2026-10-30T17:00:00Z',
            new: true,
            loadingShare: null,
            trend: FEW,
            examples: [],
        },
    ],
    modules: [
        { key: 'GTA5.exe', drops: 18, share: 0.28, signatures: 1 },
        { key: null, drops: 46, share: 0.72, signatures: 2 },
    ],
    builds: [
        { key: '3095', drops: 49, share: 0.77, signatures: 2 },
        { key: null, drops: 15, share: 0.23, signatures: 1 },
    ],
    keptFrom: '2026-10-23T19:00:00Z',
};

/**
 * The same signatures for a statistics viewer: the code name's text was shared by a single player,
 * so no example of it reaches them.
 */
export const SIGNATURES_STATS: SignatureTable = {
    ...SIGNATURES,
    signatures: SIGNATURES.signatures.map((entry) => (entry.id === 12 ? { ...entry, examples: [] } : entry)),
};

export const RESOURCES: ResourceTable = {
    range: '7d',
    kicks: 28,
    resources: [
        {
            resource: 'afk_kick',
            kicks: 21,
            share: 0.75,
            players: 17,
            trend: STEADY,
            reasons: [
                { template: 'AFK for {n} minutes', kicks: 19 },
                { template: 'Kicked by {player}', kicks: 2 },
            ],
        },
        { resource: null, kicks: 7, share: 0.25, players: 6, trend: FEW, reasons: [] },
    ],
    keptFrom: '2026-10-23T19:00:00Z',
};

/** The same kickers for a statistics viewer: never what a resource wrote. */
export const RESOURCES_STATS: ResourceTable = {
    ...RESOURCES,
    resources: RESOURCES.resources.map((entry) => ({ ...entry, reasons: null })),
};

export const CHANGES: DropChanges = {
    range: '7d',
    snapshots: false,
    events: [
        {
            at: '2026-10-30T14:02:07Z',
            kind: 'incident',
            run: 41,
            generation: 2,
            buildFrom: null,
            buildTo: null,
            playersDropped: null,
            drops: 38,
            hang: false,
            incidentCategories: [
                { category: 'timeout', drops: 31 },
                { category: 'crash', drops: 7 },
            ],
            hitchMs: 2_400,
            playersOnline: 120,
            beforeAfter: [],
            newSignatures: [],
        },
        {
            at: '2026-10-29T21:40:00Z',
            kind: 'serverCrash',
            run: 40,
            generation: 1,
            buildFrom: null,
            buildTo: null,
            playersDropped: 54,
            drops: null,
            hang: false,
            incidentCategories: [],
            hitchMs: null,
            playersOnline: null,
            beforeAfter: [],
            newSignatures: [],
        },
        {
            at: '2026-10-29T22:00:00Z',
            kind: 'build',
            run: 41,
            generation: 1,
            buildFrom: 16500,
            buildTo: 17000,
            playersDropped: null,
            drops: null,
            hang: false,
            incidentCategories: [],
            hitchMs: null,
            playersOnline: null,
            beforeAfter: [{ category: 'crash', trend: { ...RISING, change: 0.48, now: 4.6, before: 3.1 } }],
            newSignatures: ['load:c0000005:cars'],
        },
    ],
};

export const UPTIME: DropUptime = {
    range: '7d',
    bins: [
        { fromHours: 0, toHours: 1, drops: 9, playedMs: 120 * HOUR, rate: 7.5, verdict: 'usual' },
        { fromHours: 1, toHours: 3, drops: 18, playedMs: 260 * HOUR, rate: 6.9, verdict: 'usual' },
        { fromHours: 3, toHours: 6, drops: 24, playedMs: 330 * HOUR, rate: 7.3, verdict: 'usual' },
        { fromHours: 6, toHours: 12, drops: 41, playedMs: 420 * HOUR, rate: 9.8, verdict: 'usual' },
        { fromHours: 12, toHours: null, drops: 39, playedMs: 260 * HOUR, rate: 15, verdict: 'higher' },
    ],
    keptFrom: '2026-10-23T19:00:00Z',
};

export const LOG: DropLog = {
    rows: [
        {
            id: 901,
            at: '2026-10-30T18:40:00Z',
            player: { id: 7, license: LICENSE, name: 'Ruby Vex' },
            category: 'crash',
            kind: null,
            resource: null,
            signatureId: 12,
            signature: 'hash:GTA5_b3095.exe+1A2B3C',
            label: 'kilo-ohio-lemon',
            reason: null,
            lengthMs: 2 * HOUR + 14 * 60_000,
            loading: false,
            run: 41,
            generation: 2,
            synthesized: false,
            incident: false,
        },
        {
            id: 900,
            at: '2026-10-30T18:02:00Z',
            player: null,
            category: 'kick',
            kind: 'resource',
            resource: 'afk_kick',
            signatureId: null,
            signature: null,
            label: null,
            reason: 'AFK for 31 minutes <script>alert(1)</script>',
            lengthMs: HOUR,
            loading: false,
            run: 41,
            generation: 2,
            synthesized: false,
            incident: false,
        },
        {
            id: 899,
            at: '2026-10-29T21:40:00Z',
            player: null,
            category: 'serverCrash',
            kind: 'crash',
            resource: null,
            signatureId: null,
            signature: null,
            label: null,
            reason: null,
            lengthMs: 40 * 60_000,
            loading: null,
            run: 40,
            generation: 1,
            synthesized: true,
            incident: false,
        },
    ],
    next: '1761760800000.899',
};

export const SETTINGS: DropsSettings = { sessionDays: 90 };
