import { describe, expect, it } from 'vitest';

import { lineTokens } from './cfgSyntax';

const plain = (text: string) => [{ text, secret: false }];

describe('the cfg preview colours', () => {
    it('should mark the command, its strings and numbers', () => {
        expect(lineTokens(plain('sv_maxclients 64'))).toEqual([
            { text: 'sv_maxclients', kind: 'command' },
            { text: ' ', kind: 'plain' },
            { text: '64', kind: 'number' },
        ]);
        expect(lineTokens(plain('endpoint_add_tcp "0.0.0.0:30120"'))).toEqual([
            { text: 'endpoint_add_tcp', kind: 'command' },
            { text: ' ', kind: 'plain' },
            { text: '"0.0.0.0:30120"', kind: 'string' },
        ]);
    });

    it('should leave a comment as one, to the end of the line', () => {
        expect(lineTokens(plain('# Lab server, main config'))).toEqual([
            { text: '# Lab server, main config', kind: 'comment' },
        ]);
        expect(lineTokens(plain('ensure chat // the chat'))).toEqual([
            { text: 'ensure', kind: 'command' },
            { text: ' chat ', kind: 'plain' },
            { text: '// the chat', kind: 'comment' },
        ]);
    });

    it('should keep an empty token where Observer left a secret out', () => {
        expect(
            lineTokens([
                { text: 'sv_licenseKey ', secret: false },
                { text: '', secret: true },
            ]),
        ).toEqual([
            { text: 'sv_licenseKey', kind: 'command' },
            { text: ' ', kind: 'plain' },
            { text: '', kind: 'secret' },
        ]);
    });

    it('should read a number that starts a line as its command', () => {
        expect(lineTokens(plain('3095'))).toEqual([{ text: '3095', kind: 'command' }]);
    });
});
