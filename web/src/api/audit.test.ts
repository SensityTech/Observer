import { describe, expect, it } from 'vitest';

import { auditSearch, exportFilename } from './audit';

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
