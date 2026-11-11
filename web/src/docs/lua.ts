export type LuaKind = 'comment' | 'string' | 'keyword' | 'number' | 'plain';
/** A piece of Lua, keyed by where it starts in its text. */
export type LuaToken = { at: number; text: string; kind: LuaKind };

// A comment to the end of its line, a quoted string, a keyword, a number. Long brackets are left
// plain: the contract's examples have none.
const PIECE =
    /(--.*)|('(?:[^'\\\n]|\\.)*'|"(?:[^"\\\n]|\\.)*")|\b(and|break|do|else|elseif|end|false|for|function|goto|if|in|local|nil|not|or|repeat|return|then|true|until|while)\b|\b(\d+(?:\.\d+)?)\b/g;

/** Lua as the documentation colours it: comments, strings, keywords and numbers. */
export const luaTokens = (code: string): LuaToken[] => {
    const tokens: LuaToken[] = [];
    let at = 0;
    for (const match of code.matchAll(PIECE)) {
        if (match.index > at) {
            tokens.push({ at, text: code.slice(at, match.index), kind: 'plain' });
        }
        const [text, comment, string, keyword] = match;
        const kind =
            comment !== undefined
                ? 'comment'
                : string !== undefined
                  ? 'string'
                  : keyword !== undefined
                    ? 'keyword'
                    : 'number';
        tokens.push({ at: match.index, text, kind });
        at = match.index + text.length;
    }
    if (at < code.length) {
        tokens.push({ at, text: code.slice(at), kind: 'plain' });
    }
    return tokens;
};
