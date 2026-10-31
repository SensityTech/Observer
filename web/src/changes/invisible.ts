// Bidirectional controls reorder what follows them and zero-width characters hide in a word, so code
// can read one way and run another (Trojan Source). Code and the names and paths the server reports
// show each one as an escape, `⟨U+202E⟩`, never as itself.

const INVISIBLE = /[\u061C\u200B-\u200F\u202A-\u202E\u2060\u2066-\u2069\uFEFF]/g;
const ZERO_WIDTH = /[\u200B-\u200D\u2060\uFEFF]/;
const BYTE_ORDER_MARK = '\uFEFF';

export type InvisibleKind = 'bidi' | 'zeroWidth';

/** One of those characters, by its code point: `U+202E`. */
export type Invisible = { code: string; kind: InvisibleKind };

/** A run of text, or one of those characters, by where it starts. */
export type Piece = { at: number; text: string; invisible: Invisible | null };

const invisible = (char: string): Invisible => ({
    code: `U+${(char.codePointAt(0) ?? 0).toString(16).toUpperCase().padStart(4, '0')}`,
    kind: ZERO_WIDTH.test(char) ? 'zeroWidth' : 'bidi',
});

/**
 * The text cut around those characters. A byte order mark that opens a file is what it says it is,
 * and stays in the text.
 */
export const invisiblePieces = (text: string, opensFile = false): Piece[] => {
    const pieces: Piece[] = [];
    let from = 0;
    for (const match of text.matchAll(INVISIBLE)) {
        if (opensFile && match.index === 0 && match[0] === BYTE_ORDER_MARK) {
            continue;
        }
        if (match.index > from) {
            pieces.push({ at: from, text: text.slice(from, match.index), invisible: null });
        }
        pieces.push({ at: match.index, text: match[0], invisible: invisible(match[0]) });
        from = match.index + match[0].length;
    }
    if (from < text.length) {
        pieces.push({ at: from, text: text.slice(from), invisible: null });
    }
    return pieces;
};

export const holdsInvisible = (text: string, opensFile = false) =>
    invisiblePieces(text, opensFile).some((piece) => piece.invisible !== null);

/** How one of those characters shows: `⟨U+202E⟩`. */
export const escapeOf = ({ code }: Invisible) => `⟨${code}⟩`;

/** A name or a path with each of those characters written out: `config⟨U+202E⟩gol.lua`. */
export const escapeInvisible = (text: string) => text.replace(INVISIBLE, (char) => escapeOf(invisible(char)));
