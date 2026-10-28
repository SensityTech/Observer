import { describe, expect, it } from 'vitest';

import { en } from '../i18n/en';
import { formatNumber, formatTimestamp } from '../i18n/format';
import { auditDetails } from './details';

const format = {
    number: (value: number) => formatNumber('en', value),
    timestamp: (rfc3339: string) => formatTimestamp(new Date(rfc3339)),
};

describe('auditDetails', () => {
    it('should name permissions rather than show their mask', () => {
        expect(auditDetails(en, format, { userId: 2, added: '2', removed: '0', permissions: '103' })).toBe(
            'Account id: 2 · Added: View the console · Removed: none · ' +
                'Permissions: View the dashboard, View the console, View the audit log',
        );
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

    it('should show what it does not know as the log stores it', () => {
        expect(auditDetails(en, format, { trigger: 'manual', refusal: 'unheard_of', extra: { a: 1 } })).toBe(
            'trigger: manual · Reason: unheard_of · extra: {"a":1}',
        );
        expect(auditDetails(en, format, { reason: 'gremlins' })).toBe('Reason: gremlins');
    });
});
