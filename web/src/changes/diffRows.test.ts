import { describe, expect, it } from 'vitest';

import type { DiffLine } from '../api/generated/DiffLine';
import type { Hunk } from '../api/generated/Hunk';
import { hunkHeader, splitRows, textRows, unifiedRows } from './diffRows';

const line = (kind: DiffLine['kind'], from: number | null, to: number | null): DiffLine => ({
    kind,
    from,
    to,
    segments: [{ text: `${kind} ${from ?? to}`, changed: false, redacted: false }],
});

/** Two lines replaced by three, between unchanged ones, then one more removed. */
const HUNK: Hunk = {
    fromStart: 10,
    fromLines: 5,
    toStart: 10,
    toLines: 5,
    lines: [
        line('context', 10, 10),
        line('removed', 11, null),
        line('removed', 12, null),
        line('added', null, 11),
        line('added', null, 12),
        line('added', null, 13),
        line('context', 13, 14),
        line('removed', 14, null),
    ],
};

/** Each row as `before|after` line numbers, or the hunk's header. */
const shape = (rows: ReturnType<typeof splitRows>) =>
    rows.map((row) => {
        if (row.type === 'hunk') {
            return hunkHeader(row.hunk);
        }
        if (row.type === 'pair') {
            return `${row.before?.from ?? '-'}|${row.after?.to ?? '-'}`;
        }
        return `${row.line.from ?? '-'}|${row.line.to ?? '-'}`;
    });

describe('diff rows', () => {
    it('should put each line on its own row under the header of its hunk', () => {
        expect(shape(unifiedRows([HUNK]))).toEqual([
            '@@ -10,5 +10,5 @@',
            '10|10',
            '11|-',
            '12|-',
            '-|11',
            '-|12',
            '-|13',
            '13|14',
            '14|-',
        ]);
    });

    it('should set removed lines beside the lines that replaced them, the longer run facing blanks', () => {
        expect(shape(splitRows([HUNK]))).toEqual([
            '@@ -10,5 +10,5 @@',
            '10|10',
            '11|11',
            '12|12',
            '-|13',
            '13|14',
            '14|-',
        ]);
    });

    it('should start a new pair once lines are removed after others were added', () => {
        const hunk = {
            ...HUNK,
            lines: [line('added', null, 1), line('removed', 1, null), line('added', null, 2)],
        };

        expect(shape(splitRows([hunk]))).toEqual(['@@ -10,5 +10,5 @@', '-|1', '1|2']);
    });

    it('should number the lines of a whole file from the first of its page', () => {
        const rows = textRows({ first: 501, lines: [[], []], total: 900, page: 1, pages: 2 });

        expect(shape(rows)).toEqual(['501|501', '502|502']);
    });
});
