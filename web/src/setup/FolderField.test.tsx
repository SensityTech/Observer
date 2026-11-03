import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { FolderEntry } from '../api/generated/FolderEntry';
import type { FolderListing } from '../api/generated/FolderListing';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { FolderField } from './FolderField';

const words = en.serverSetup.folder;

const entry = (name: string, path: string, extra: Partial<FolderEntry> = {}): FolderEntry => ({
    name,
    path,
    kind: 'folder',
    link: false,
    resources: null,
    cfg: null,
    folders: null,
    inside: [],
    ...extra,
});

/** `D:\` as the listing gives it for `Se`: a folder holding a server and txAdmin's data, and one Observer may not read. */
const LISTING: FolderListing = {
    path: 'D:\\',
    parent: null,
    problem: null,
    pathProblem: null,
    entries: [
        entry('Sensity', 'D:\\Sensity', {
            folders: 3,
            inside: [
                entry('FXServer', 'D:\\Sensity\\FXServer', {
                    kind: 'server',
                    resources: 412,
                    cfg: 'server.cfg',
                }),
                entry('txData', 'D:\\Sensity\\txData', { kind: 'txAdminData' }),
            ],
        }),
        entry('Setup', 'D:\\Setup', { kind: 'unreadable' }),
    ],
    truncated: false,
};

/** Answers each listing with `LISTING`, and says what each asked. */
const serve = () => {
    const asked: unknown[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (_url: string, init: RequestInit = {}) => {
            asked.push(JSON.parse(String(init.body)));
            return Response.json(LISTING);
        }),
    );
    return asked;
};

function Harness({ onChoose }: { onChoose: (path: string) => void }) {
    const [value, setValue] = useState('');
    return <FolderField value={value} error={undefined} onChange={setValue} onChoose={onChoose} />;
}

const field = () => {
    const onChoose = vi.fn();
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const view = render(
        <QueryClientProvider client={client}>
            <I18nProvider>
                <Harness onChoose={onChoose} />
            </I18nProvider>
        </QueryClientProvider>,
    );
    const input = view.getByRole('combobox', { name: words.label }) as HTMLInputElement;
    const press = (key: string) => fireEvent.keyDown(input, { key });
    const options = () => view.queryAllByRole('option');
    /** Types `text`, then waits for the folders it lists. */
    const type = async (text: string) => {
        fireEvent.change(input, { target: { value: text } });
        await waitFor(() => expect(options().length).toBeGreaterThan(0));
    };
    return { ...view, input, press, options, type, onChoose };
};

describe('FolderField', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it("should list the typed folder's folders that start with the rest, servers inside first", async () => {
        const asked = serve();
        const { input, options, type, getByRole } = field();

        expect(input.getAttribute('aria-expanded')).toBe('false');
        await type('D:\\Se');

        expect(asked).toEqual([{ path: 'D:\\', prefix: 'Se' }]);
        expect(input.getAttribute('aria-expanded')).toBe('true');
        expect(input.getAttribute('aria-autocomplete')).toBe('list');
        expect(document.getElementById(input.getAttribute('aria-controls') ?? '')).toBe(
            getByRole('listbox', { name: words.suggestions }),
        );
        const texts = options().map((option) => option.textContent);
        expect(texts).toEqual([
            `Sensity\\FXServer${words.kinds.server}${words.serverDetail(412, 'server.cfg')}`,
            `Sensity${words.folderDetail(3, true)}`,
            `Sensity\\txData${words.kinds.txAdminData}${words.txAdminDetail}`,
            `Setup${words.kinds.unreadable}`,
        ]);
        expect(options()[3]?.getAttribute('aria-disabled')).toBe('true');
    });

    it('should move through the options with the arrows, round the list, past a folder it may not read', async () => {
        serve();
        const { input, options, press, type } = field();
        await type('D:\\Se');
        const active = () => document.getElementById(input.getAttribute('aria-activedescendant') ?? '');

        press('ArrowDown');
        expect(active()).toBe(options()[0]);
        expect(options()[0]?.getAttribute('aria-selected')).toBe('true');
        press('ArrowUp');
        expect(active()).toBe(options()[2]);
        press('ArrowDown');
        expect(active()).toBe(options()[0]);
    });

    it('should complete a server with Tab and stop there, and go into any other folder', async () => {
        serve();
        const { input, options, press, type } = field();
        await type('D:\\Se');

        press('Tab');
        expect(input.value).toBe('D:\\Sensity\\FXServer');
        expect(input.getAttribute('aria-expanded')).toBe('false');

        await type('D:\\Se');
        press('ArrowDown');
        press('ArrowDown');
        press('Tab');
        expect(input.value).toBe('D:\\Sensity\\');
        expect(options().length).toBeGreaterThan(0);
    });

    it('should choose the option picked with Enter, or else the path typed, and have it checked', async () => {
        serve();
        const { input, onChoose, press, type } = field();
        await type('D:\\Se');

        press('ArrowDown');
        press('ArrowDown');
        press('Enter');
        expect(onChoose).toHaveBeenLastCalledWith('D:\\Sensity');
        expect(input.value).toBe('D:\\Sensity');
        expect(input.getAttribute('aria-expanded')).toBe('false');

        fireEvent.change(input, { target: { value: ' D:\\Elsewhere ' } });
        press('Enter');
        expect(onChoose).toHaveBeenLastCalledWith('D:\\Elsewhere');
    });

    it('should close the list on Escape and keep what was typed', async () => {
        serve();
        const { input, press, type } = field();
        await type('D:\\Se');

        press('Escape');

        expect(input.getAttribute('aria-expanded')).toBe('false');
        expect(input.value).toBe('D:\\Se');
    });
});
