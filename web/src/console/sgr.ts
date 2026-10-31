// FXServer colours its output with SGR sequences (`ESC [ … m`), its channel prefixes with the 256
// colour palette, and resources' `^n` codes arrive already turned into SGR. Observer keeps nothing
// else of the escapes. The panel reads them into spans of the console's closed palette (eight
// tones, one --term-* token each), which components render as text, never as markup.

/** A colour of the console's palette: `bright` is the ink, `dim` the console's grey. */
export type Tone = 'red' | 'green' | 'yellow' | 'blue' | 'magenta' | 'cyan' | 'bright' | 'dim';

export type Span = { text: string; tone: Tone | null; bold: boolean };

type Style = { tone: Tone | null; bold: boolean; dim: boolean };

const PLAIN: Style = { tone: null, bold: false, dim: false };

const BASIC: (Tone | null)[] = ['dim', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', null];
const BRIGHT: (Tone | null)[] = ['dim', 'red', 'green', 'yellow', 'blue', 'magenta', 'cyan', 'bright'];

/** The nearest tone of an RGB colour, by hue; greys go to the text colour or the dim one. */
const toneOf = (red: number, green: number, blue: number): Tone | null => {
    const max = Math.max(red, green, blue);
    const min = Math.min(red, green, blue);
    if (max === 0 || (max - min) / max < 0.25) {
        return max < 128 ? 'dim' : null;
    }
    const delta = max - min;
    let hue =
        max === red
            ? ((green - blue) / delta) % 6
            : max === green
              ? (blue - red) / delta + 2
              : (red - green) / delta + 4;
    hue = (hue * 60 + 360) % 360;
    if (hue < 20 || hue >= 345) {
        return 'red';
    }
    if (hue < 70) {
        return 'yellow';
    }
    if (hue < 160) {
        return 'green';
    }
    if (hue < 200) {
        return 'cyan';
    }
    if (hue < 250) {
        return 'blue';
    }
    return 'magenta';
};

/** The intensity of one step of xterm's colour cube. */
const level = (step: number) => (step === 0 ? 0 : 55 + step * 40);

/** xterm's 256 colours: the 16 of the terminal, a 6×6×6 cube, then 24 greys. */
const paletteTone = (index: number): Tone | null => {
    if (index < 8) {
        return BASIC[index] ?? null;
    }
    if (index < 16) {
        return BRIGHT[index - 8] ?? null;
    }
    if (index < 232) {
        const cube = index - 16;
        return toneOf(level(Math.floor(cube / 36)), level(Math.floor(cube / 6) % 6), level(cube % 6));
    }
    return index < 244 ? 'dim' : null;
};

/** Applies one SGR sequence's parameters to the style in force. */
const apply = (style: Style, parameters: number[]): Style => {
    let next = style;
    for (let index = 0; index < parameters.length; index += 1) {
        const code = parameters[index] ?? 0;
        if (code === 0) {
            next = PLAIN;
        } else if (code === 1) {
            next = { ...next, bold: true };
        } else if (code === 2) {
            next = { ...next, dim: true };
        } else if (code === 22) {
            next = { ...next, bold: false, dim: false };
        } else if (code >= 30 && code <= 37) {
            next = { ...next, tone: BASIC[code - 30] ?? null };
        } else if (code === 39) {
            next = { ...next, tone: null };
        } else if (code >= 90 && code <= 97) {
            next = { ...next, tone: BRIGHT[code - 90] ?? null };
        } else if (code === 38 || code === 48) {
            // An extended colour: `5;n` or `2;r;g;b`. Backgrounds are read past, never painted.
            const mode = parameters[index + 1];
            if (mode === 5) {
                if (code === 38) {
                    next = { ...next, tone: paletteTone(parameters[index + 2] ?? 0) };
                }
                index += 2;
            } else if (mode === 2) {
                if (code === 38) {
                    const [red = 0, green = 0, blue = 0] = parameters.slice(index + 2, index + 5);
                    next = { ...next, tone: toneOf(red, green, blue) };
                }
                index += 4;
            } else {
                index += 1;
            }
        }
    }
    return next;
};

const toneFor = (style: Style): Tone | null => (style.dim ? 'dim' : style.tone);

const ESC = String.fromCodePoint(0x1b);
// Built from a string: the escape is the one control character the console's text may hold.
const SGR = new RegExp(`${ESC}\\[([0-9;]*)m`, 'g');

/** The line cut into spans of one style each, its escapes gone. */
export const parseSgr = (text: string): Span[] => {
    const spans: Span[] = [];
    let style = PLAIN;
    let start = 0;
    const push = (end: number) => {
        // A stray escape, which Observer never lets through, is dropped rather than shown.
        const piece = text.slice(start, end).replaceAll(ESC, '');
        if (piece === '') {
            return;
        }
        const tone = toneFor(style);
        const last = spans.at(-1);
        if (last !== undefined && last.tone === tone && last.bold === style.bold) {
            last.text += piece;
        } else {
            spans.push({ text: piece, tone, bold: style.bold });
        }
    };
    for (const match of text.matchAll(SGR)) {
        push(match.index);
        const written = match[1] ?? '';
        const parameters = written === '' ? [0] : written.split(';').map((part) => Number(part) || 0);
        style = apply(style, parameters);
        start = match.index + match[0].length;
    }
    push(text.length);
    return spans;
};

/** The line as a reader sees it, for search. */
export const plainText = (text: string) => text.replace(SGR, '').replaceAll(ESC, '');
