import { cleanup, fireEvent } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ToolsView } from '../api/generated/ToolsView';
import type { ToolView } from '../api/generated/ToolView';
import { en } from '../i18n/en';
import { openSettings } from './fixtures';

const words = en.tools.settings;
const items = en.settings.items;
const bar = en.settings.bar;

const viewOf = (tool: Partial<ToolView> = {}): ToolsView => ({
    tools: [
        {
            tool: 'duplicateArchetypes',
            settings: { atStart: 'off', differences: 'identical', escrowed: 'skip', excluded: [] },
            version: 2,
            lastRun: null,
            lastReport: null,
            open: [],
            elsewhere: [],
            unreachable: [],
            closed: [],
            backupBytes: 0,
            staged: null,
            ...tool,
        },
    ],
});

const answer = (status: number, body?: unknown) =>
    new Response(body === undefined ? null : JSON.stringify(body), {
        status,
        headers: { 'content-type': 'application/json' },
    });

/** Observer's answers by method and route; every request made is kept, with its body. */
const serve = (replies: Record<string, unknown | (() => unknown)>) => {
    const asked: { route: string; body: unknown }[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string, init: RequestInit) => {
            const route = `${init.method ?? 'GET'} ${new URL(input, 'http://localhost').pathname.replace(/^\/api/, '')}`;
            const body = typeof init.body === 'string' ? JSON.parse(init.body) : undefined;
            asked.push({ route, body });
            const found = replies[route];
            const reply = typeof found === 'function' ? (found as () => unknown)() : found;
            return reply instanceof Response
                ? reply.clone()
                : answer(reply === undefined ? 404 : 200, reply ?? { code: 'not_found', params: {} });
        }),
    );
    return asked;
};

describe('Server tools settings', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should save the settings over their version, and say when another session changed them', async () => {
        const saved = viewOf({
            settings: {
                atStart: 'report',
                differences: 'identical',
                escrowed: 'skip',
                excluded: ['qb-props'],
            },
            version: 3,
        });
        const replies = [saved, answer(409, { code: 'conflict', params: {} })];
        const asked = serve({
            'GET /tools': () => (replies.length === 2 ? viewOf() : saved),
            'PUT /tools/duplicate-archetypes/settings': () => replies.shift(),
        });
        const { getByLabelText, getByRole, findByText, getByText } = await openSettings('/settings/tools');

        fireEvent.click(getByLabelText(words.atStartModes.report));
        fireEvent.change(getByLabelText(items.toolExcluded.label), {
            target: { value: ' qb-props\n\nqb-props\n' },
        });
        expect(getByText(bar.changes(2))).not.toBeNull();
        expect(getByText(`· ${bar.applies([en.settings.appliesWhen.nextRun])}`)).not.toBeNull();
        fireEvent.click(getByRole('button', { name: bar.save }));

        expect(await findByText(words.saved)).not.toBeNull();
        expect(asked.find((request) => request.route.startsWith('PUT'))?.body).toEqual({
            settings: {
                atStart: 'report',
                differences: 'identical',
                escrowed: 'skip',
                excluded: ['qb-props'],
            },
            version: 2,
        });

        fireEvent.click(getByLabelText(words.escrowedModes.allow));
        fireEvent.click(getByRole('button', { name: bar.save }));
        expect(await findByText(words.changedElsewhere)).not.toBeNull();
        expect(asked.findLast((request) => request.route.startsWith('PUT'))?.body).toMatchObject({
            version: 3,
        });
    });

    it('should refuse an excluded name holding a slash before sending it', async () => {
        const asked = serve({ 'GET /tools': viewOf() });
        const { getByLabelText, getByRole, getByText } = await openSettings('/settings/tools');

        fireEvent.change(getByLabelText(items.toolExcluded.label), { target: { value: '[qb]/qb-props' } });
        fireEvent.click(getByRole('button', { name: bar.save }));

        expect(getByText(words.excludedInvalid)).not.toBeNull();
        expect(asked.some((request) => request.route.startsWith('PUT'))).toBe(false);
    });

    it('should name each choice by its setting, and explain it', async () => {
        serve({ 'GET /tools': viewOf() });
        const { getByRole } = await openSettings('/settings/tools');

        const atStart = getByRole('radiogroup', { name: items.toolAtStart.label });
        expect(document.getElementById(atStart.getAttribute('aria-describedby') ?? '')?.textContent).toBe(
            items.toolAtStart.about,
        );
    });
});
