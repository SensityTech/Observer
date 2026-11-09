import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Markdown, parseBlocks, parseInline } from './Markdown';

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

    it('keeps a fenced block as it is, and joins the lines of a paragraph', () => {
        const blocks = parseBlocks('one\ntwo\n\n```\n# not a heading\n```\n1. first\n2. second');

        expect(blocks).toMatchObject([
            { kind: 'paragraph', text: 'one two' },
            { kind: 'code', text: '# not a heading' },
            { kind: 'list', ordered: true, items: [{ text: 'first' }, { text: 'second' }] },
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

describe('Markdown', () => {
    it('draws a pull request and a comparison on GitHub as GitHub names them', () => {
        render(<Markdown source={NOTES} />);

        const pull = screen.getByRole('link', { name: '#45' });
        expect(pull.getAttribute('href')).toBe('https://github.com/SensityTech/Observer/pull/45');
        expect(pull.getAttribute('rel')).toBe('noreferrer noopener');
        expect(screen.getByRole('link', { name: 'v0.1.0...v0.2.0' })).toBeTruthy();
        expect(screen.getByRole('heading', { name: "What's Changed" })).toBeTruthy();
        expect(screen.getAllByRole('listitem')).toHaveLength(3);
    });

    it('shows markup written in the notes as text', () => {
        const { container } = render(<Markdown source={'<img src=x onerror="alert(1)"> <b>bold</b>'} />);

        expect(container.querySelector('img')).toBeNull();
        expect(container.querySelector('b')).toBeNull();
        expect(container.textContent).toContain('<img src=x onerror="alert(1)">');
    });
});
