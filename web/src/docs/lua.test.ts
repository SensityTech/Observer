import { describe, expect, it } from 'vitest';

import { integrationDocs } from './fixtures';
import { luaTokens } from './lua';

describe('luaTokens', () => {
    it('tells comments, strings, keywords and numbers from the rest', () => {
        const code = "local bridge = GetConvar('observer_bridge', '') -- empty, or 4";

        expect(luaTokens(code).map(({ text, kind }) => [text, kind])).toEqual([
            ['local', 'keyword'],
            [' bridge = GetConvar(', 'plain'],
            ["'observer_bridge'", 'string'],
            [', ', 'plain'],
            ["''", 'string'],
            [') ', 'plain'],
            ['-- empty, or 4', 'comment'],
        ]);
    });

    it('takes a keyword inside a name or a string for what it is', () => {
        expect(
            luaTokens("endsAt = 'in the end' and 30 or nil").map(({ text, kind }) => [text, kind]),
        ).toEqual([
            ['endsAt = ', 'plain'],
            ["'in the end'", 'string'],
            [' ', 'plain'],
            ['and', 'keyword'],
            [' ', 'plain'],
            ['30', 'number'],
            [' ', 'plain'],
            ['or', 'keyword'],
            [' ', 'plain'],
            ['nil', 'keyword'],
        ]);
    });

    it('gives back every character of the example resource, in order', () => {
        for (const { text } of integrationDocs().example) {
            const tokens = luaTokens(text);

            expect(tokens.map((token) => token.text).join('')).toBe(text);
            expect(new Set(tokens.map((token) => token.at)).size).toBe(tokens.length);
        }
    });
});
