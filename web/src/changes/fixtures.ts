import type { ChangeCounts } from '../api/generated/ChangeCounts';
import type { ChangesSettingsView } from '../api/generated/ChangesSettingsView';
import type { Comparison } from '../api/generated/Comparison';
import type { FileChange } from '../api/generated/FileChange';
import type { FileDiff } from '../api/generated/FileDiff';
import type { ResourceChange } from '../api/generated/ResourceChange';
import type { ResourceDetail } from '../api/generated/ResourceDetail';
import type { SnapshotSummary } from '../api/generated/SnapshotSummary';
import type { StartList } from '../api/generated/StartList';

// A server restarted twice since its baseline: an update of qb-core, an inventory whose files changed
// under the same version, a build bump, and a webhook in a config that someone rotated.

export const BASELINE: SnapshotSummary = {
    id: 1,
    kind: 'start',
    takenAt: '2026-10-29T08:00:00Z',
    cause: 'requested',
    by: 'ada',
    state: 'complete',
    partialReason: null,
    baseline: true,
    build: 25770,
    gameBuild: 3095,
    files: 10_412,
    bytes: 4_812_000_000,
    counts: null,
    crashed: true,
    runStarted: '2026-10-29T07:59:58Z',
    generation: 1,
};

export const RESTART: SnapshotSummary = {
    ...BASELINE,
    id: 2,
    takenAt: '2026-10-30T04:00:00Z',
    cause: 'scheduled',
    by: null,
    baseline: false,
    crashed: false,
    counts: {
        added: 0,
        removed: 0,
        updated: 1,
        sameVersion: 0,
        renamed: 0,
        moved: 0,
        didNotStart: 0,
        checking: 0,
        unchanged: 124,
        files: 3,
        environment: [],
    },
};

const LATEST_COUNTS: ChangeCounts = {
    added: 1,
    removed: 1,
    updated: 2,
    sameVersion: 1,
    renamed: 0,
    moved: 0,
    didNotStart: 1,
    checking: 0,
    unchanged: 120,
    files: 9,
    environment: ['build', 'cfg'],
};

export const LATEST: SnapshotSummary = {
    ...RESTART,
    id: 3,
    takenAt: '2026-10-31T14:02:00Z',
    cause: 'restart',
    by: 'ada',
    build: 26100,
    counts: LATEST_COUNTS,
};

export const STARTS: StartList = { starts: [LATEST, RESTART, BASELINE], next: null, check: null };

const resource = (
    change: Partial<ResourceChange> & Pick<ResourceChange, 'name' | 'path' | 'change'>,
): ResourceChange => ({
    fromPath: null,
    changed: true,
    fromVersion: null,
    toVersion: null,
    duplicate: false,
    escrowed: false,
    files: { added: 0, removed: 0, modified: 0 },
    lines: null,
    by: [],
    ...change,
});

export const INVENTORY = resource({
    name: 'qb-inventory',
    path: 'resources/[qb]/qb-inventory',
    change: 'sameVersion',
    fromVersion: '2.4.1',
    toVersion: '2.4.1',
    escrowed: true,
    files: { added: 0, removed: 0, modified: 3 },
    lines: { added: 4, removed: 2 },
    by: ['bob'],
});

export const COMPARISON: Comparison = {
    from: RESTART,
    to: LATEST,
    counts: LATEST_COUNTS,
    environment: {
        build: { from: 25770, to: 26100 },
        gameBuild: null,
        game: null,
        settings: [],
        arguments: [],
        cfg: { files: [], changed: 2, lines: { added: 5, removed: 1 }, started: ['qb-housing'], stopped: [] },
        attributions: [
            { action: 'build_selected', by: 'ada', at: '2026-10-31T13:40:00Z', details: { build: 26100 } },
        ],
    },
    resources: [
        INVENTORY,
        resource({
            name: 'qb-core',
            path: 'resources/[qb]/qb-core',
            change: 'updated',
            fromVersion: '1.2.0',
            toVersion: '1.3.0',
            files: { added: 2, removed: 1, modified: 12 },
            lines: { added: 120, removed: 33 },
        }),
        resource({
            name: 'ox_lib',
            path: 'resources/[ox]/ox_lib',
            change: 'updated',
            fromVersion: '3.26.0',
            toVersion: '3.27.0',
            files: { added: 0, removed: 0, modified: 4 },
        }),
        resource({
            name: 'jg-dealerships',
            path: 'resources/[paid]/jg-dealerships',
            change: 'added',
            toVersion: '1.1.0',
        }),
        resource({
            name: 'old-hud',
            path: 'resources/[ui]/old-hud',
            change: 'removed',
            fromVersion: '0.9.0',
        }),
        resource({ name: 'chat', path: 'resources/[cfx]/chat', change: 'didNotStart', changed: false }),
    ],
    live: [
        { at: '2026-10-31T15:02:11Z', name: 'chat', change: 'restarted', by: 'bob', command: 'restart chat' },
        { at: '2026-10-31T15:30:40Z', name: 'qb-housing', change: 'stopped', by: null, command: null },
    ],
    changedWhileStarting: 0,
};

const file = (change: Partial<FileChange> & Pick<FileChange, 'path' | 'kind' | 'change'>): FileChange => ({
    fromPath: null,
    fromSize: 2_048,
    toSize: 2_210,
    fromSha256: null,
    toSha256: null,
    lines: null,
    flags: [],
    hiddenBy: null,
    kept: true,
    writtenBy: null,
    writtenAt: null,
    ...change,
});

const root = INVENTORY.path;

export const CLIENT = file({
    path: `${root}/client/main.lua`,
    kind: 'text',
    change: 'modified',
    lines: { added: 3, removed: 1 },
});

export const CONFIG = file({
    path: `${root}/config.lua`,
    kind: 'text',
    change: 'modified',
    lines: { added: 1, removed: 1 },
    flags: ['afterStart'],
});

export const DETAIL: ResourceDetail = {
    change: INVENTORY,
    fromManifest: null,
    toManifest: null,
    fromGit: null,
    toGit: null,
    packages: [],
    files: [
        CLIENT,
        CONFIG,
        file({ path: `${root}/fxmanifest.lua`, kind: 'text', change: 'unchanged', toSize: 2_048 }),
        file({
            path: `${root}/stream/bag.ydr`,
            kind: 'binary',
            change: 'modified',
            fromSize: 1_258_291,
            toSize: 1_258_291,
            fromSha256: 'a3f1c09b5e7d2f84c6b1e0d9a7f35c28e4b6d0193f7a2c5e8d1b4f6a9c0e3d72',
            toSha256: '5c8e2d71f4a09b3e6d1c7f28a5b0e94d3c6f1a82b7e5d09c4a3f6b1e8d2c7a05',
        }),
    ],
    duplicates: [],
};

/** config.lua's rotated webhook: the redaction changed, the word around it did not. */
export const CONFIG_DIFF: FileDiff = {
    file: CONFIG,
    diff: {
        added: 1,
        removed: 1,
        lineEndingsOnly: false,
        fromTotal: 40,
        toTotal: 40,
        page: 0,
        pages: 1,
        hunks: [
            {
                fromStart: 11,
                fromLines: 3,
                toStart: 11,
                toLines: 3,
                lines: [
                    {
                        kind: 'context',
                        from: 11,
                        to: 11,
                        segments: [{ text: 'Config.Logs = {', changed: false, redacted: false }],
                    },
                    {
                        kind: 'removed',
                        from: 12,
                        to: null,
                        segments: [
                            { text: '    webhook = "', changed: false, redacted: false },
                            { text: '[REDACTED]', changed: true, redacted: true },
                            { text: '",', changed: false, redacted: false },
                        ],
                    },
                    {
                        kind: 'added',
                        from: null,
                        to: 12,
                        segments: [
                            { text: '    webhook = "', changed: false, redacted: false },
                            { text: '[REDACTED]', changed: true, redacted: true },
                            { text: '",', changed: false, redacted: false },
                        ],
                    },
                    {
                        kind: 'context',
                        from: 13,
                        to: 13,
                        segments: [{ text: '}', changed: false, redacted: false }],
                    },
                ],
            },
        ],
    },
};

export const SETTINGS: ChangesSettingsView = {
    settings: {
        enabled: true,
        extensions: ['lua', 'js', 'json', 'cfg'],
        excluded: ['cache'],
        hidden: ['resources/[core]/my_bot/config/secrets.lua', '*.secret'],
        maxText: 1024 * 1024,
        budget: 2_000_000_000,
        retentionDays: 180,
        hashRate: 64_000_000,
        verifyDays: 7,
        startEvent: true,
    },
    hiddenMatches: [1, 0],
    excludedMatches: [0],
    stored: 412 * 1024 * 1024,
};
