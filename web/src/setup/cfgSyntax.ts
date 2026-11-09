import type { CfgSpan } from '../api/generated/CfgSpan';

export type TokenKind = 'command' | 'string' | 'number' | 'comment' | 'plain' | 'secret';
export type Token = { text: string; kind: TokenKind };

// Spaces, a comment to the end of the line, a quoted string (perhaps left open), a number, a word.
const PIECE = /(\s+)|(#.*|\/\/.*)|("(?:[^"\\]|\\.)*"?)|(-?\d+(?:\.\d+)?(?![^\s"]))|([^\s"]+)/gu;

/**
 * A line of a cfg as the preview colours it: the command it starts with, quoted strings, numbers
 * and comments, and an empty token where Observer left a secret out.
 */
export const lineTokens = (spans: readonly CfgSpan[]): Token[] => {
    const tokens: Token[] = [];
    const push = (text: string, kind: TokenKind) => {
        const last = tokens.at(-1);
        if (last !== undefined && last.kind === kind && kind !== 'secret') {
            last.text += text;
        } else {
            tokens.push({ text, kind });
        }
    };
    let commanded = false;
    let comment = false;
    for (const span of spans) {
        if (span.secret) {
            push('', 'secret');
            commanded = true;
            continue;
        }
        if (comment) {
            push(span.text, 'comment');
            continue;
        }
        for (const [, space, note, quoted, number, word] of span.text.matchAll(PIECE)) {
            if (space !== undefined) {
                push(space, 'plain');
            } else if (note !== undefined) {
                push(note, 'comment');
                comment = true;
            } else if (quoted !== undefined) {
                push(quoted, 'string');
                commanded = true;
            } else if (number !== undefined && commanded) {
                push(number, 'number');
            } else if (word !== undefined || number !== undefined) {
                push(word ?? number ?? '', commanded ? 'plain' : 'command');
                commanded = true;
            }
        }
    }
    return tokens;
};

/** The tokens of a line, each keyed by where it starts in it: a secret counts one place. */
export const keyedTokens = (spans: readonly CfgSpan[]) => {
    let at = 0;
    return lineTokens(spans).map((token) => {
        const key = at;
        at += token.kind === 'secret' ? 1 : token.text.length;
        return { token, key };
    });
};
