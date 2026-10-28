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
                buildPath: { before: null, after: 'C:\\FXServer\\build' },
                onesync: { before: 'on', after: 'legacy' },
                autoStart: { before: false, after: true },
                extraArguments: { before: [], after: [{ name: 'sv_enforceGameBuild', value: '3258' }] },
            }),
        ).toBe(
            'Build folder: not set → C:\\FXServer\\build · OneSync: on → legacy · Start with Observer: no → yes · ' +
                'Extra arguments: none → sv_enforceGameBuild 3258',
        );
        expect(auditDetails(en, format, { ownerOnly: true })).toBe('Owner only: yes');
    });

    it('should show what it does not know as the log stores it', () => {
        expect(auditDetails(en, format, { reason: 'manual', refusal: 'unheard_of', extra: { a: 1 } })).toBe(
            'reason: manual · Reason: unheard_of · extra: {"a":1}',
        );
    });
});
