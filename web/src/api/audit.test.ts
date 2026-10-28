import { describe, expect, it } from 'vitest';

import { auditSearch, exportFilename, matchesFilter, withLive } from './audit';
import type { AuditEntry } from './generated/AuditEntry';

const entry = (id: number, changes: Partial<AuditEntry> = {}): AuditEntry => ({
    id,
    at: '2026-10-28T13:00:00.000Z',
    actorKind: 'user',
    actorId: 2,
    actorName: 'Grace',
    ip: '203.0.113.7',
    userAgent: 'Firefox',
    action: 'login_succeeded',
    target: 'grace',
    outcome: 'success',
    details: {},
    ...changes,
});

describe('auditSearch', () => {
    it('should send only the filters that are set', () => {
        expect(auditSearch({})).toBe('');
        expect(auditSearch({ actor: '', action: undefined, outcome: 'denied' })).toBe('?outcome=denied');
    });

    it('should encode the values for a query string', () => {
        expect(
            auditSearch({
                actor: 'ada',
                action: 'user_created',
                from: '2026-10-28T13:00:00.000Z',
                before: 412,
                limit: 50,
            }),
        ).toBe('?actor=ada&action=user_created&from=2026-10-28T13%3A00%3A00.000Z&before=412&limit=50');
    });
});

describe('exportFilename', () => {
    it('should name the file after the local moment of the export', () => {
        expect(exportFilename(new Date(2026, 9, 28, 14, 2, 7))).toBe('observer-audit-20261028-140207.csv');
        expect(exportFilename(new Date(2027, 0, 3, 9, 5, 0))).toBe('observer-audit-20270103-090500.csv');
    });
});

describe('matchesFilter', () => {
    it('should keep a live entry only where the API would list it', () => {
        const seen = entry(9);

        expect(matchesFilter(seen, {})).toBe(true);
        expect(matchesFilter(seen, { actor: 'grace', action: 'login_succeeded', outcome: 'success' })).toBe(
            true,
        );
        expect(matchesFilter(seen, { actor: 'ada' })).toBe(false);
        expect(matchesFilter(entry(9, { actorName: null }), { actor: 'grace' })).toBe(false);
        expect(matchesFilter(seen, { action: 'logout' })).toBe(false);
        expect(matchesFilter(seen, { outcome: 'denied' })).toBe(false);
        expect(matchesFilter(seen, { from: '2026-10-28T13:00:00.000Z' })).toBe(true);
        expect(matchesFilter(seen, { to: '2026-10-28T13:00:00.000Z' })).toBe(false);
        expect(matchesFilter(seen, { from: '2026-10-28T12:00:00Z', to: '2026-10-28T14:00:00Z' })).toBe(true);
    });
});

const ids = (entries: AuditEntry[]) => entries.map((each) => each.id);

describe('withLive', () => {
    it('should put live entries above the pages and drop those a reload brought', () => {
        expect(ids(withLive([entry(12), entry(11)], [entry(10), entry(8)]))).toEqual([12, 11, 10, 8]);
        expect(ids(withLive([entry(12), entry(11)], [entry(11), entry(10)]))).toEqual([12, 11, 10]);
        expect(ids(withLive([entry(3)], []))).toEqual([3]);
    });
});
