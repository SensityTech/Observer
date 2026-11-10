import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { CfgDocument } from '../api/generated/CfgDocument';
import type { CfgReview } from '../api/generated/CfgReview';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { CfgPage } from './CfgPage';

vi.mock('@tanstack/react-router', () => ({
    Link: ({ to, children }: { to: string; children: ReactNode }) => <a href={to}>{children}</a>,
}));

const TEXT = [
    'endpoint_add_tcp "0.0.0.0:30120"',
    'endpoint_add_udp "0.0.0.0:30120"',
    'sv_hostname "Lab"',
    'sv_licenseKey <<secret-1>>',
    'ensure chat',
    '#ensure [xmas]',
    'ensure rivals',
    '',
].join('\n');

const REVIEW: CfgReview = {
    outline: {
        settings: [
            { line: 3, command: null, name: 'sv_hostname', value: 'Lab', start: 12, end: 17, alone: true },
            { line: 4, command: null, name: 'sv_licenseKey', value: null, start: 14, end: 26, alone: true },
        ],
        endpoints: [
            { line: 1, protocol: 'tcp', address: '0.0.0.0:30120', start: 17, end: 32, alone: true },
            { line: 2, protocol: 'udp', address: '0.0.0.0:30120', start: 17, end: 32, alone: true },
        ],
        resources: [
            { line: 5, name: 'chat', enabled: true, alone: true, missing: false },
            { line: 6, name: '[xmas]', enabled: false, alone: true, missing: false },
            { line: 7, name: 'rivals', enabled: true, alone: true, missing: true },
        ],
        truncated: false,
    },
    findings: [
        {
            kind: 'resourceMissing',
            severity: 'warning',
            file: 'server.cfg',
            line: 7,
            params: { name: 'rivals' },
        },
    ],
    added: 0,
    removed: 0,
};

const DOCUMENT: CfgDocument = {
    file: 'server.cfg',
    text: TEXT,
    base: 'a'.repeat(64),
    bytes: 150,
    modifiedAt: '2026-11-09T21:35:00Z',
    secrets: 1,
    check: REVIEW,
    versions: 2,
};

type Call = { method: string; path: string; body: unknown };

/** The page over an API that answers each route with what `answers` gives it, and keeps the calls. */
const page = (answers: Partial<Record<string, (body: unknown) => Response>> = {}) => {
    const calls: Call[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (url: string, init?: RequestInit) => {
            const method = init?.method ?? 'GET';
            const path = url.replace('/api/servers/default', '');
            const body: unknown = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
            calls.push({ method, path, body });
            const answer = answers[`${method} ${path}`];
            if (answer !== undefined) {
                return answer(body);
            }
            if (path === '/cfg/check') {
                return Response.json({ ...REVIEW, added: 1, removed: 1 });
            }
            return Response.json(DOCUMENT);
        }),
    );
    const view = render(
        <QueryClientProvider client={new QueryClient()}>
            <I18nProvider>
                <CfgPage />
            </I18nProvider>
        </QueryClientProvider>,
    );
    const sent = (method: string, path: string) =>
        calls.filter((call) => call.method === method && call.path === path);
    return { ...view, calls, sent };
};

const words = en.cfg;

describe('CfgPage', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should show each field with the line it is written on, and a secret as kept rather than shown', async () => {
        const { findByLabelText, getByText, getByRole, getAllByRole, queryByDisplayValue } = page();

        const name = await findByLabelText(words.guided.fields.sv_hostname);
        expect((name as HTMLInputElement).value).toBe('Lab');
        expect(getByText('sv_hostname · line 3')).not.toBeNull();
        expect(getByText('endpoint_add_tcp, endpoint_add_udp · lines 1 and 2')).not.toBeNull();
        expect(getByText(words.guided.secretKept)).not.toBeNull();
        expect(queryByDisplayValue('<<secret-1>>')).toBeNull();
        expect(getByText('not in the file: a save adds a sets tags line')).not.toBeNull();
        const rows = getAllByRole('listitem').filter((row) => within(row).queryByRole('switch') !== null);
        expect(
            rows.map((row) => [
                row.textContent,
                (within(row).getByRole('switch') as HTMLInputElement).checked,
            ]),
        ).toEqual([
            ['5chat', true],
            ['6[xmas]', false],
            [`7rivals${words.guided.missing}`, true],
        ]);
        expect(getByRole('heading', { level: 1 }).textContent).toBe('server.cfg');
        expect(getByText(words.bar.unchanged)).not.toBeNull();
        expect((getByRole('button', { name: words.bar.save }) as HTMLButtonElement).disabled).toBe(true);
    });

    it('should save the text with what the fields changed written into it, and nothing else', async () => {
        const saved = { ...DOCUMENT, base: 'b'.repeat(64), versions: 3 };
        const { findByLabelText, getByRole, findByText, sent } = page({
            'PUT /cfg': () => Response.json(saved),
        });

        fireEvent.change(await findByLabelText(words.guided.fields.sv_hostname), {
            target: { value: 'Café RP' },
        });
        fireEvent.click(getByRole('switch', { name: words.guided.starts('[xmas]') }));
        fireEvent.click(getByRole('switch', { name: words.guided.starts('rivals') }));
        fireEvent.change(getByRole('textbox', { name: words.guided.add }), { target: { value: '[maps]' } });
        fireEvent.click(getByRole('button', { name: words.guided.addAction }));
        expect(await findByText(words.bar.changed(1), undefined, { timeout: 2000 })).not.toBeNull();
        fireEvent.click(getByRole('button', { name: words.bar.save }));

        expect(await findByText(words.done.saved)).not.toBeNull();
        expect(sent('PUT', '/cfg').map((call) => call.body)).toEqual([
            {
                base: DOCUMENT.base,
                text: [
                    'endpoint_add_tcp "0.0.0.0:30120"',
                    'endpoint_add_udp "0.0.0.0:30120"',
                    'sv_hostname "Café RP"',
                    'sv_licenseKey <<secret-1>>',
                    'ensure chat',
                    'ensure [xmas]',
                    '#ensure rivals',
                    'ensure [maps]',
                    '',
                ].join('\n'),
            },
        ]);
        expect(getByRole('button', { name: words.bar.versions(3) })).not.toBeNull();
    });

    it('should read the text again as it is typed, and comment out a line a check names', async () => {
        const { findByRole, getByRole, findByText, sent } = page();

        fireEvent.click(await findByRole('button', { name: words.views.file }));
        const text = getByRole('textbox', { name: words.file.text('server.cfg') }) as HTMLTextAreaElement;
        expect(text.value).toBe(TEXT);
        expect(await findByText(en.serverSetup.cfg.kinds.resourceMissing({ name: 'rivals' }))).not.toBeNull();
        fireEvent.change(text, { target: { value: TEXT.replace('"Lab"', '"Lab 2"') } });

        await waitFor(() => expect(sent('POST', '/cfg/check')).toHaveLength(1), { timeout: 2000 });
        expect(sent('POST', '/cfg/check')[0]?.body).toEqual({ text: TEXT.replace('"Lab"', '"Lab 2"') });
        fireEvent.click(await findByRole('button', { name: words.checks.commentOut }));

        expect(text.value).toBe(TEXT.replace('"Lab"', '"Lab 2"').replace('ensure rivals', '#ensure rivals'));
    });

    it('should say the file changed on disk when a save is refused for it, and read it again', async () => {
        const { findByLabelText, getByRole, findByText, sent } = page({
            'PUT /cfg': () => Response.json({ code: 'cfg_changed' }, { status: 409 }),
        });

        fireEvent.change(await findByLabelText(words.guided.fields.sv_hostname), {
            target: { value: 'New' },
        });
        fireEvent.click(getByRole('button', { name: words.bar.save }));

        expect(await findByText(words.conflict)).not.toBeNull();
        expect(sent('GET', '/cfg')).toHaveLength(1);
        fireEvent.click(getByRole('button', { name: words.readAgain }));
        await waitFor(() => expect(sent('GET', '/cfg')).toHaveLength(2));
    });

    it('should never write a value FXServer would cut, and say so by the field', async () => {
        const { findByLabelText, findByRole, getByRole } = page();

        fireEvent.change(await findByLabelText(words.guided.fields.sv_hostname), {
            target: { value: 'a"; quit' },
        });

        expect((await findByRole('alert')).textContent).toBe(words.guided.unwritable);
        expect((getByRole('button', { name: words.bar.save }) as HTMLButtonElement).disabled).toBe(true);
    });

    it('should say why a cfg is not edited from here', async () => {
        const { findByText } = page({
            'GET /cfg': () =>
                Response.json({ code: 'cfg_not_editable', params: { reason: 'notUtf8' } }, { status: 409 }),
        });

        expect(await findByText(words.refused.notUtf8)).not.toBeNull();
    });
});
