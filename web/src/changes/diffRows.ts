import type { DiffLine } from '../api/generated/DiffLine';
import type { Hunk } from '../api/generated/Hunk';
import type { TextPage } from '../api/generated/TextPage';
import { holdsInvisible } from './invisible';

/** What one row of a code block shows: a hunk's header, a line, or two lines side by side. */
export type DiffRow =
    | { type: 'hunk'; key: string; hunk: Hunk }
    | { type: 'line'; key: string; line: DiffLine }
    | { type: 'pair'; key: string; before: DiffLine | null; after: DiffLine | null };

/** `@@ -12,7 +12,9 @@`, as diff tools head a hunk. */
export const hunkHeader = (hunk: Hunk) =>
    `@@ -${hunk.fromStart},${hunk.fromLines} +${hunk.toStart},${hunk.toLines} @@`;

/** One row a line, each hunk under its header. */
export const unifiedRows = (hunks: Hunk[]): DiffRow[] =>
    hunks.flatMap((hunk, index): DiffRow[] => [
        { type: 'hunk', key: `${index}`, hunk },
        ...hunk.lines.map((line, at): DiffRow => ({ type: 'line', key: `${index}:${at}`, line })),
    ]);

/**
 * The two versions side by side: an unchanged line on both sides, and a run of removed lines beside
 * the run of added lines that replaced it, the longer one facing blanks.
 */
export const splitRows = (hunks: Hunk[]): DiffRow[] =>
    hunks.flatMap((hunk, index) => {
        const rows: DiffRow[] = [{ type: 'hunk', key: `${index}`, hunk }];
        const pair = (before: DiffLine | null, after: DiffLine | null) =>
            rows.push({ type: 'pair', key: `${index}:${rows.length}`, before, after });
        let removed: DiffLine[] = [];
        let added: DiffLine[] = [];
        const flush = () => {
            for (let at = 0; at < Math.max(removed.length, added.length); at += 1) {
                pair(removed[at] ?? null, added[at] ?? null);
            }
            removed = [];
            added = [];
        };
        for (const line of hunk.lines) {
            if (line.kind === 'removed') {
                if (added.length > 0) {
                    flush();
                }
                removed.push(line);
            } else if (line.kind === 'added') {
                added.push(line);
            } else {
                flush();
                pair(line, line);
            }
        }
        flush();
        return rows;
    });

/** A page of a whole file, each line numbered from the page's first. */
export const textRows = (page: TextPage): DiffRow[] =>
    page.lines.map((segments, index) => {
        const number = page.first + index;
        return {
            type: 'line',
            key: `${number}`,
            line: { kind: 'context', from: number, to: number, segments },
        };
    });

/** Whether a line opens its file, in either version: where a byte order mark belongs. */
export const isFirstLine = (line: DiffLine) => line.from === 1 || line.to === 1;

/** Whether a line shown holds a character that reorders or hides code. */
export const holdInvisible = (rows: DiffRow[]) =>
    rows.some((row) => {
        const lines = row.type === 'line' ? [row.line] : row.type === 'pair' ? [row.before, row.after] : [];
        return lines.some(
            (line) =>
                line !== null &&
                line.segments.some(
                    (segment, index) =>
                        !segment.redacted && holdsInvisible(segment.text, index === 0 && isFirstLine(line)),
                ),
        );
    });
