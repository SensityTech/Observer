import { describe, expect, it } from 'vitest';

import { parseSgr, plainText } from './sgr';

const ESC = String.fromCodePoint(0x1b);
const sgr = (parameters: string) => `${ESC}[${parameters}m`;

describe('parseSgr', () => {
    it('should read a channel prefix of FXServer 35245 in the palette tone of its colour', () => {
        const line = `${sgr('38;5;73')}[ citizen-server-impl] ${sgr('0')}${sgr('0')}Authenticating server license key...`;

        expect(parseSgr(line)).toEqual([
            { text: '[ citizen-server-impl] ', tone: 'cyan', bold: false },
            { text: 'Authenticating server license key...', tone: null, bold: false },
        ]);
    });

    it('should map the 256 colours FXServer gives its channels to the closed palette', () => {
        const channels: [number, string][] = [
            [171, 'magenta'],
            [83, 'green'],
            [66, 'cyan'],
            [161, 'magenta'],
            [129, 'magenta'],
            [73, 'cyan'],
            [186, 'yellow'],
            [75, 'blue'],
        ];
        for (const [colour, tone] of channels) {
            expect(parseSgr(`${sgr(`38;5;${colour}`)}x`)[0]?.tone, String(colour)).toBe(tone);
        }
    });

    it("should read the basic and bright colours a resource's ^n codes became", () => {
        // `^1red^7 plain ^2green^0 reset`, as FXServer prints it.
        const line = `${sgr('91')}red ${sgr('0')} plain ${sgr('32')}green ${sgr('97')} reset`;

        expect(parseSgr(line)).toEqual([
            { text: 'red ', tone: 'red', bold: false },
            { text: ' plain ', tone: null, bold: false },
            { text: 'green ', tone: 'green', bold: false },
            { text: ' reset', tone: 'bright', bold: false },
        ]);
    });

    it('should keep the weight apart from the colour and dim with grey', () => {
        expect(
            parseSgr(`${sgr('1;33')}warn${sgr('22')}ing ${sgr('2')}quiet${sgr('22')} ${sgr('90')}grey`),
        ).toEqual([
            { text: 'warn', tone: 'yellow', bold: true },
            { text: 'ing ', tone: 'yellow', bold: false },
            { text: 'quiet', tone: 'dim', bold: false },
            { text: ' ', tone: 'yellow', bold: false },
            { text: 'grey', tone: 'dim', bold: false },
        ]);
    });

    it('should read past backgrounds and true colours without painting a background', () => {
        expect(parseSgr(`${sgr('41;48;5;200;38;2;250;20;20')}alarm${sgr('39;49')} back`)).toEqual([
            { text: 'alarm', tone: 'red', bold: false },
            { text: ' back', tone: null, bold: false },
        ]);
    });

    it('should treat an empty sequence as a reset and drop a stray escape', () => {
        expect(parseSgr(`${sgr('35')}a${ESC}[mb${ESC}c`)).toEqual([
            { text: 'a', tone: 'magenta', bold: false },
            { text: 'bc', tone: null, bold: false },
        ]);
        expect(parseSgr('')).toEqual([]);
    });

    it('should leave markup as text', () => {
        expect(parseSgr('<img src=x onerror=alert(1)>')).toEqual([
            { text: '<img src=x onerror=alert(1)>', tone: null, bold: false },
        ]);
    });
});

describe('plainText', () => {
    it('should be the line without its colours', () => {
        expect(plainText(`${sgr('38;5;83')}[resources] ${sgr('0')}Started resource ${ESC}chat`)).toBe(
            '[resources] Started resource chat',
        );
    });
});
