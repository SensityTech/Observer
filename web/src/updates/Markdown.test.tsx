import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Markdown } from './Markdown';

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

    it('draws a table, and a link to a place beside the notes as its words', () => {
        const { container } = render(
            <Markdown source={'| File | For |\n|---|---|\n| `a.zip` | [Windows](docs/windows) |'} />,
        );

        expect(container.querySelector('th:last-child')?.textContent).toBe('For');
        expect(container.querySelector('td:last-child')?.textContent).toBe('Windows');
        expect(container.querySelector('a')).toBeNull();
    });
});
