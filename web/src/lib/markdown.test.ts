import { describe, expect, it } from 'vitest';

import { parseBlocks, parseInline, plainText } from './markdown';

const NOTES = [
    'Observer 0.2.0, built from 0123456789ab.',
    '',
    '- `observer-0.2.0-windows-x86_64.zip`: Windows 10 or Server 2016 and later, x64.',
    '- `SHA256SUMS`: check the archives with `sha256sum -c SHA256SUMS`.',
    '',
    "## What's Changed",
    '* Install updates from the panel by @someone in https://github.com/SensityTech/Observer/pull/45',
    '',
    '**Full Changelog**: https://github.com/SensityTech/Observer/compare/v0.1.0...v0.2.0',
].join('\r\n');

describe('parseBlocks', () => {
    it('reads the paragraphs, lists and headings of release notes', () => {
        expect(parseBlocks(NOTES).map((block) => block.kind)).toMatchObject([
            'paragraph',
            'list',
            'heading',
            'list',
            'paragraph',
        ]);
    });

    it('keeps a fenced block as it is, with the language it names, and joins the lines of a paragraph', () => {
        const blocks = parseBlocks('one\ntwo\n\n```lua\n# not a heading\n```\n1. first\n2. second');

        expect(blocks).toMatchObject([
            { kind: 'paragraph', text: 'one two' },
            { kind: 'code', text: '# not a heading', lang: 'lua' },
            { kind: 'list', ordered: true, items: [{ text: 'first' }, { text: 'second' }] },
        ]);
    });

    it('reads a table, a bar inside a code span staying in its cell', () => {
        const blocks = parseBlocks(
            [
                'Before.',
                '| Field | Type | Meaning |',
                '|---|:--|---|',
                '| `at` | integer | When it happens. |',
                '| `kind` | string | `a|b`, or nothing |',
                '| `short` |',
                'After.',
            ].join('\n'),
        );

        expect(blocks).toMatchObject([
            { kind: 'paragraph', text: 'Before.' },
            {
                kind: 'table',
                head: [{ text: 'Field' }, { text: 'Type' }, { text: 'Meaning' }],
                rows: [
                    { cells: [{ text: '`at`' }, { text: 'integer' }, { text: 'When it happens.' }] },
                    { cells: [{ text: '`kind`' }, { text: 'string' }, { text: '`a|b`, or nothing' }] },
                    { cells: [{ text: '`short`' }, { text: '' }, { text: '' }] },
                ],
            },
            { kind: 'paragraph', text: 'After.' },
        ]);
    });

    it('takes lines between bars for text when no rule follows them', () => {
        expect(parseBlocks('| not | a table |\nnext')).toMatchObject([
            { kind: 'paragraph', text: '| not | a table | next' },
        ]);
    });
});

describe('parseInline', () => {
    it('reads code, emphasis and links', () => {
        expect(parseInline('**Full** `code` *mild* [docs](https://example.com/a)')).toMatchObject([
            { kind: 'strong', children: [{ kind: 'text', text: 'Full' }] },
            { kind: 'text', text: ' ' },
            { kind: 'code', text: 'code' },
            { kind: 'text', text: ' ' },
            { kind: 'em', children: [{ kind: 'text', text: 'mild' }] },
            { kind: 'text', text: ' ' },
            {
                kind: 'link',
                href: 'https://example.com/a',
                children: [{ kind: 'text', text: 'docs' }],
            },
        ]);
    });

    it('never makes a link of an address that is not http or https', () => {
        expect(parseInline('[click](javascript:alert(1))')).toMatchObject([{ kind: 'text', text: 'click)' }]);
        expect(parseInline('[file](file:///etc/passwd)')).toMatchObject([{ kind: 'text', text: 'file' }]);
        expect(parseInline('[elsewhere](//example.com/a)')).toMatchObject([
            { kind: 'text', text: 'elsewhere' },
        ]);
    });

    it('tells a place in the same text and a path beside it from an address', () => {
        expect(parseInline('See [Bans](#bans) and [`observer_example`](observer_example).')).toMatchObject([
            { kind: 'text', text: 'See ' },
            { kind: 'local', target: '#bans', children: [{ kind: 'text', text: 'Bans' }] },
            { kind: 'text', text: ' and ' },
            {
                kind: 'local',
                target: 'observer_example',
                children: [{ kind: 'code', text: 'observer_example' }],
            },
            { kind: 'text', text: '.' },
        ]);
    });

    it('leaves a snake_case name alone and a bare address without its last stop', () => {
        expect(parseInline('observer_bridge and https://example.com/x.')).toMatchObject([
            { kind: 'text', text: 'observer_bridge and ' },
            {
                kind: 'link',
                href: 'https://example.com/x',
                children: [{ kind: 'text', text: 'https://example.com/x' }],
            },
            { kind: 'text', text: '.' },
        ]);
    });
});

describe('plainText', () => {
    it('says a text without its markup', () => {
        expect(plainText(parseInline('`observer:shuttingDown` is **raised** [here](#events)'))).toBe(
            'observer:shuttingDown is raised here',
        );
    });
});
