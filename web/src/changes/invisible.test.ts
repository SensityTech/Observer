import { describe, expect, it } from 'vitest';

import type { DiffLine } from '../api/generated/DiffLine';
import { holdInvisible, textRows } from './diffRows';
import { escapeInvisible, invisiblePieces } from './invisible';

// Written by code point: a source file holding them would hide them from its own review.
const RLO = String.fromCodePoint(0x202e);
const PDI = String.fromCodePoint(0x2069);
const ZWSP = String.fromCodePoint(0x200b);
const BOM = String.fromCodePoint(0xfeff);

describe('invisiblePieces', () => {
    it('should cut a line around each bidirectional control and zero-width character', () => {
        expect(invisiblePieces(`if (isAdmin${RLO} ${PDI}) {`)).toEqual([
            { at: 0, text: 'if (isAdmin', invisible: null },
            { at: 11, text: RLO, invisible: { code: 'U+202E', kind: 'bidi' } },
            { at: 12, text: ' ', invisible: null },
            { at: 13, text: PDI, invisible: { code: 'U+2069', kind: 'bidi' } },
            { at: 14, text: ') {', invisible: null },
        ]);
        expect(invisiblePieces(`ad${ZWSP}min`).map((piece) => piece.invisible?.kind ?? null)).toEqual([
            null,
            'zeroWidth',
            null,
        ]);
    });

    it('should leave a byte order mark that opens the file, and escape one anywhere else', () => {
        expect(invisiblePieces(`${BOM}local x = 1`, true)).toEqual([
            { at: 0, text: `${BOM}local x = 1`, invisible: null },
        ]);
        expect(invisiblePieces(`${BOM}local x = 1`)).toHaveLength(2);
        expect(invisiblePieces(`local${BOM} x`, true)).toHaveLength(3);
    });
});

describe('escapeInvisible', () => {
    it('should write each one out in a name or a path, and leave other text alone', () => {
        expect(escapeInvisible(`resources/[qb]/config${RLO}aul.lua`)).toBe(
            'resources/[qb]/config⟨U+202E⟩aul.lua',
        );
        expect(escapeInvisible('ox_lib')).toBe('ox_lib');
        expect(escapeInvisible('qb-hébergement')).toBe('qb-hébergement');
    });
});

/** A page of a whole file, its lines from `first`. */
const page = (lines: string[], first = 1) =>
    textRows({
        first,
        lines: lines.map((text) => [{ text, changed: false, redacted: false }]),
        total: 40,
        page: 0,
        pages: 1,
    });

describe('holdInvisible', () => {
    it("should find one in any line shown, but not a file's own byte order mark", () => {
        expect(holdInvisible(page([`${BOM}Config = {}`, 'return Config']))).toBe(false);
        expect(holdInvisible(page([`${BOM}Config = {}`], 12))).toBe(true);
        expect(holdInvisible(page(['Config = {}', `-- ${RLO}}`]))).toBe(true);
    });

    it('should not look inside a hidden secret, whose text is never shown', () => {
        const line: DiffLine = {
            kind: 'added',
            from: null,
            to: 4,
            segments: [{ text: `x${RLO}`, changed: true, redacted: true }],
        };
        expect(holdInvisible([{ type: 'line', key: '0', line }])).toBe(false);
    });
});
