import { describe, expect, it } from 'vitest';

import { en } from '../i18n/en';
import { formatBytes, formatNumber, formatTimestamp } from '../i18n/format';
import { auditDetails } from './details';

const format = {
    number: (value: number) => formatNumber('en', value),
    bytes: (bytes: number) => formatBytes('en', bytes),
    timestamp: (rfc3339: string) => formatTimestamp(new Date(rfc3339)),
};

describe('auditDetails', () => {
    it('should name permissions rather than show their mask', () => {
        expect(auditDetails(en, format, { userId: 2, added: '2', removed: '0', permissions: '103' })).toBe(
            'Account id: 2 · Added: View the console · Removed: none · ' +
                'Permissions: View the dashboard, View the console, View the audit log',
        );
    });

    it('should give the size of a build as the panel shows sizes', () => {
        expect(auditDetails(en, format, { build: 38004, bytes: 196_642_542 })).toBe(
            'Build: 38004 · Size: 188 MB',
        );
    });

    it('should write builds as they are numbered and word what happened to them', () => {
        expect(auditDetails(en, format, { from: 37900, to: 35245, reason: 'crashed' })).toBe(
            'From build: 37900 · To build: 35245 · Reason: crashed',
        );
        expect(auditDetails(en, format, { build: 37900, from: 35245, apply: 'nextStart' })).toBe(
            'Build: 37900 · From build: 35245 · Applied: At the next start',
        );
        expect(
            auditDetails(en, format, {
                build: 37872,
                kind: 'install',
                step: 'validate',
                reason: 'signature',
            }),
        ).toBe('Build: 37872 · Kind: Install · Step: Validating · Reason: not signed by Rockstar Games');
        expect(
            auditDetails(en, format, { branches: { before: [], after: ['feature/a', 'feature/b'] } }),
        ).toBe('Test branches: not set → feature/a, feature/b');
    });

    it('should show times in local time and counts as numbers', () => {
        const first = '2026-10-28T08:01:27.884Z';
        expect(auditDetails(en, format, { network: '203.0.113.7', failures: 1200, firstAt: first })).toBe(
            `Network: 203.0.113.7 · Failures: 1,200 · First: ${formatTimestamp(new Date(first))}`,
        );
    });

    it('should word what the schedule did and why', () => {
        const at = '2026-10-29T03:00:00.000Z';
        expect(
            auditDetails(en, format, {
                at,
                source: 'oneOff',
                reason: 'serverBusy',
                state: 'stopping',
                lateBySeconds: 420,
            }),
        ).toBe(
            `At: ${formatTimestamp(new Date(at))} · From: a single restart · ` +
                'Reason: FXServer was busy · State: Stopping · Late by: 420 s',
        );
        expect(
            auditDetails(en, format, {
                times: { before: [], after: ['04:00', '16:00'] },
                mode: { before: 'fixedTimes', after: 'interval' },
                scheduled: { source: 'schedule', plannedBy: 'ada' },
            }),
        ).toBe(
            'Times: not set → 04:00, 16:00 · Mode: Fixed times → Interval · ' +
                'On schedule: (From: the schedule, Planned by: ada)',
        );
    });

    it('should word the schedule settings in minutes, a zone left unset as the host one, and what a change dropped', () => {
        expect(
            auditDetails(en, format, {
                intervalMinutes: { before: 360, after: 420 },
                skipAfterStart: { before: 120, after: 60 },
                warnings: { before: [30, 15], after: [15, 5, 1] },
                timeZone: { before: null, after: 'America/New_York' },
                dropped: [{ kind: 'skip', by: 'ada' }],
            }),
        ).toBe(
            'Interval: 360 min → 420 min · Skip after a start: 120 min → 60 min · ' +
                'Warnings: 30 min, 15 min → 15 min, 5 min, 1 min · ' +
                "Time zone: the host's zone → America/New_York · Dropped: (Kind: a skip, By: ada)",
        );
    });

    it('should word refused attempts from the catalog', () => {
        expect(
            auditDetails(en, format, {
                attempted: 'user_deleted',
                refusal: 'hierarchy',
                permission: 'usersManage',
                repeated: 3,
            }),
        ).toBe(
            'Attempted: User deleted · Reason: An account holding more than they do · ' +
                'Permission: Manage users · Repeated: 3',
        );
    });

    it('should show a changed setting as its value before and after', () => {
        expect(
            auditDetails(en, format, {
                onesync: { before: 'on', after: 'legacy' },
                autoStart: { before: null, after: true },
            }),
        ).toBe('OneSync: on → legacy · Start with Observer: not set → yes');
        expect(auditDetails(en, format, { ownerOnly: true })).toBe('Owner only: yes');
        expect(
            auditDetails(en, format, {
                hangDetection: { before: true, after: false },
                hangTimeoutSeconds: { before: 360, after: 900 },
            }),
        ).toBe('Hang detection: yes → no · Restart a hung server after: 6 min → 15 min');
        expect(auditDetails(en, format, { stopHoldSeconds: { before: 60, after: 120 } })).toBe(
            'Wait for resources at a stop: 60 s → 120 s',
        );
    });

    it('should name the resources a stop went on without, why, and after how long', () => {
        expect(
            auditDetails(
                en,
                format,
                { resources: ['esx_vehicleshop', 'qb-inventory'], reason: 'limit', waitedSeconds: 60 },
                'server_stop_hold_expired',
            ),
        ).toBe('Resources: esx_vehicleshop, qb-inventory · Reason: their limit passed · Waited: 60 s');
        expect(
            auditDetails(en, format, {
                resources: ['qb-inventory'],
                reason: 'consoleClosed',
                waitedSeconds: 0,
            }),
        ).toBe("Resources: qb-inventory · Reason: Windows was closing Observer's console · Waited: 0 s");
        expect(auditDetails(en, format, { resources: ['qb-inventory'] }, 'server_stop_hold_skipped')).toBe(
            'Resources: qb-inventory',
        );
    });

    it('should name the step a failed launch stopped at', () => {
        expect(auditDetails(en, format, { reason: 'missingData' })).toBe('Reason: No server data folder');
        expect(auditDetails(en, format, { failures: 5 })).toBe('Failures: 5');
    });

    it('should say a hidden launch setting changed, and name the arguments set or removed', () => {
        expect(
            auditDetails(en, format, {
                buildPath: true,
                cfgFile: true,
                extraArguments: { set: ['sv_hostname', 'sv_enforceGameBuild'], removed: ['sv_projectName'] },
            }),
        ).toBe(
            'Build folder: changed · Config file: changed · ' +
                'Extra arguments: set sv_hostname, sv_enforceGameBuild; removed sv_projectName',
        );
        expect(auditDetails(en, format, { extraArguments: { set: [], removed: ['sv_hostname'] } })).toBe(
            'Extra arguments: removed sv_hostname',
        );
        expect(auditDetails(en, format, { extraArguments: { set: [], removed: [] } })).toBe(
            'Extra arguments: reordered',
        );
    });

    it('should say where a stop of Observer came from', () => {
        expect(auditDetails(en, format, { from: 'window' })).toBe('From: the host window');
        expect(auditDetails(en, format, { from: 'signal' })).toBe('From: a signal');
    });

    it('should show what it does not know as the log stores it', () => {
        expect(auditDetails(en, format, { trigger: 'manual', refusal: 'unheard_of', extra: { a: 1 } })).toBe(
            'trigger: manual · Reason: unheard_of · extra: {"a":1}',
        );
        expect(auditDetails(en, format, { reason: 'gremlins' })).toBe('Reason: gremlins');
    });

    it('should word what the diagnostics audit', () => {
        expect(
            auditDetails(en, format, {
                kind: 'shell',
                name: 'powershell.exe',
                commandLine: 'powershell -NoProfile -Command exit',
                pid: 4120,
                parent: { pid: 3312, name: 'FXServer.exe' },
                generation: 4,
                repeated: 3,
            }),
        ).toBe(
            'Kind: Security alert · Program: powershell.exe · Command line: powershell -NoProfile -Command exit · ' +
                'Pid: 4120 · Parent: FXServer.exe 3312 · Run: #4 · Repeated: 3',
        );
        expect(auditDetails(en, format, { generation: 4, unrecorded: 1_234 })).toBe(
            'Run: #4 · Command lines not kept: 1,234',
        );
        expect(
            auditDetails(en, format, { file: 'hang-4.dmp', bytes: 48_666, tookMs: 120, kind: 'dump' }),
        ).toBe('File: hang-4.dmp · Size: 47.5 KB · Took: 120 ms · Kind: a stack dump');
        expect(auditDetails(en, format, { reason: 'lowDisk', tookMs: 3 })).toBe(
            'Reason: less than 1 GB free · Took: 3 ms',
        );
        expect(auditDetails(en, format, { crash: 1, file: '5aed9068.dmp', bytes: 2_867 })).toBe(
            'Crash: 1 · File: 5aed9068.dmp · Size: 2.8 KB',
        );
        expect(auditDetails(en, format, { crash: 2, names: ['a.dmp', 'a-full.dmp'], bytes: 500 })).toBe(
            'Crash: 2 · Files: a.dmp, a-full.dmp · Size: 500 bytes',
        );
    });

    it('should word the players retentions as they were and became, and an import by its counts', () => {
        const kept = { linkDays: 365, addressDays: 90, dayDays: 400, profileDays: 730 };
        expect(
            auditDetails(en, format, { from: kept, to: { ...kept, linkDays: 1, profileDays: null } }),
        ).toBe(
            'Before: (Identifiers and names: 365 days, Address fingerprints: 90 days, Play time per day: 400 days, Profiles: 730 days) · ' +
                'After: (Identifiers and names: 1 day, Address fingerprints: 90 days, Play time per day: 400 days, Profiles: kept for good)',
        );
        expect(
            auditDetails(en, format, {
                bytes: 3_145_728,
                read: 1_204,
                written: 1_200,
                imported: 1_150,
                updated: 50,
                forgotten: 3,
                invalid: 1,
            }),
        ).toBe(
            'Size: 3 MB · Read: 1,204 · Written: 1,200 · New: 1,150 · Updated: 50 · Skipped as forgotten: 3 · Invalid: 1',
        );
    });

    it('should name the two snapshots a read of changed contents compared, either one a check of the disk, and how it showed them', () => {
        expect(
            auditDetails(
                en,
                format,
                { from: 41, to: 42, resource: 'resources/[qb]/qb-core', view: 'diff' },
                'change_contents_viewed',
            ),
        ).toBe('From snapshot: 41 · To snapshot: 42 · Resource: resources/[qb]/qb-core · Read: the diff');
        expect(auditDetails(en, format, { versions: 3, bytes: 12_288 })).toBe(
            'Versions cleared: 3 · Size: 12 KB',
        );
    });

    it('should word the change tracking settings saved whole as the ones that changed, in the units the owner typed', () => {
        const before = {
            enabled: true,
            extensions: ['lua', 'js'],
            excluded: [],
            hidden: [],
            maxText: 1_048_576,
            budget: 2_000_000_000,
            retentionDays: 180,
            hashRate: 64_000_000,
            verifyDays: 7,
            startEvent: true,
        };
        const after = {
            ...before,
            hidden: ['*.secret'],
            maxText: 2_621_440,
            budget: 4_000_000_000,
            verifyDays: 1,
            hashRate: 32_000_000,
        };

        expect(auditDetails(en, format, { before, after }, 'change_settings_changed')).toBe(
            'Contents never kept: not set → *.secret · Largest text kept: 1 MB → 2.5 MB · ' +
                'Contents kept at most: 2 GB → 4 GB · Hashing rate: 64 MB/s → 32 MB/s · ' +
                'Full verification every: 7 days → 1 day',
        );
    });

    it('should leave a file group changed as the log stores it, not as changed settings', () => {
        const before = { name: 'scripts', rules: [], reach: 'read' };
        const after = { name: 'assets', rules: [], reach: 'assets' };

        expect(auditDetails(en, format, { before, after }, 'file_group_changed')).toBe(
            `Before: ${JSON.stringify(before)} · After: ${JSON.stringify(after)}`,
        );
    });
});
