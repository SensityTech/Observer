import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ServerState } from '../api/generated/ServerState';
import type { StartWarnings } from '../api/generated/StartWarnings';
import type { ToolsView } from '../api/generated/ToolsView';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { ToolsPage } from './ToolsPage';
import { ToolWarningsPage } from './ToolWarningsPage';

vi.mock('@tanstack/react-router', () => ({
    Link: ({ to, children, className }: { to: string; children: ReactNode; className?: string }) => (
        <a href={to} className={className}>
            {children}
        </a>
    ),
}));

const server = vi.hoisted(() => ({ state: 'online' as string }));
vi.mock('../server/context', () => ({
    useServerStatus: () => ({ status: { state: server.state as ServerState }, refused: false }),
}));

const counts = {
    resources: 387,
    started: 300,
    client: 280,
    escrowed: 4,
    ytypLoaded: 100,
    ytypShadowed: 0,
    ytypUnreadable: 0,
    modelInfoLoaded: 20,
    modelInfoUnreadable: 0,
    registrations: 23_537,
    groups: 63,
    groupsFixed: 4,
    filesChanged: 1,
    removals: 4,
};

/** The tools as the API lists them: a report of 63 duplicates, 4 of them fixable. */
const tools = (staged: boolean): ToolsView =>
    ({
        tools: [
            {
                tool: 'duplicateArchetypes',
                settings: { atStart: 'off', differences: 'identical', escrowed: 'skip', excluded: [] },
                version: 2,
                lastRun: {
                    id: 7,
                    tool: 'duplicateArchetypes',
                    trigger: 'panel',
                    actor: 'ada',
                    mode: 'report',
                    startedAt: '2026-11-09T20:19:00Z',
                    finishedAt: '2026-11-09T20:19:03Z',
                    outcome: 'done',
                    summary: { elapsedMs: 3200 },
                    plan: 'a'.repeat(64),
                    report: { counts, groups: [] },
                },
                lastReport: null,
                open: [],
                elsewhere: [],
                unreachable: [],
                closed: [],
                backupBytes: 0,
                staged: staged
                    ? { plan: 'a'.repeat(64), fixes: 4, stagedAt: '2026-11-09T20:25:00Z', stagedBy: 'ada' }
                    : null,
            },
        ],
    }) as unknown as ToolsView;

const WARNINGS: StartWarnings = {
    startedAt: '2026-11-09T20:16:58Z',
    heavy: [
        { resource: 'maps', file: 'pier.ytd', tenths: 162, memory: 'physical', oversized: false },
        { resource: 'maps', file: 'dock.ydr', tenths: 495, memory: 'physical', oversized: true },
        { resource: 'cars', file: 'gt.ytd', tenths: 480, memory: 'virtual', oversized: false },
    ],
    missing: [
        { resource: 'props', kind: 'client_script', file: 'cl.lua', manifest: 'fxmanifest.lua', line: 12 },
        { resource: 'audio', kind: 'file', file: 'mix.rel', manifest: '__resource.lua', line: 3 },
    ],
    truncated: false,
};

type Answers = Partial<Record<string, (body: unknown) => unknown>>;

/** A page over an API that answers each route with what `answers` gives, the defaults otherwise. */
const shown = (page: ReactNode, answers: Answers = {}) => {
    const calls: { route: string; body: unknown }[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (url: string, init?: RequestInit) => {
            const route = `${init?.method ?? 'GET'} ${url.replace('/api', '')}`;
            const body: unknown = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined;
            calls.push({ route, body });
            const answer = answers[route];
            if (answer !== undefined) {
                return Response.json(answer(body));
            }
            if (route === 'GET /tools') {
                return Response.json(tools(false));
            }
            if (route === 'GET /servers/default/start/warnings') {
                return Response.json(WARNINGS);
            }
            if (route === 'GET /servers/default/cfg') {
                return Response.json({
                    file: 'server.cfg',
                    check: {
                        findings: [
                            { kind: 'resourceMissing', severity: 'warning', file: 'server.cfg', line: 7 },
                            { kind: 'resourceMissing', severity: 'warning', file: 'server.cfg', line: 9 },
                        ],
                    },
                });
            }
            return Response.json({ code: 'not_found' }, { status: 404 });
        }),
    );
    const view = render(
        <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
            <I18nProvider>{page}</I18nProvider>
        </QueryClientProvider>,
    );
    return { ...view, calls };
};

const words = en.tools.hub;
const card = (title: string) => {
    const heading = [...document.querySelectorAll('article h2')].find((each) => each.textContent === title);
    const article = heading?.closest('article');
    if (article === null || article === undefined) {
        throw new Error(`no ${title} card`);
    }
    return article as HTMLElement;
};

describe('ToolsPage', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        server.state = 'online';
    });

    it('should give each tool a card with what it found last and the way in', async () => {
        const { findByText, getByText } = shown(<ToolsPage />);

        expect(await findByText(words.archetypes.found(63, 4))).not.toBeNull();
        await findByText(words.heavy.found(3, 1));
        await findByText(words.cfg.unheld(2));
        expect(getByText(words.missing.found(2, 2))).not.toBeNull();
        expect(getByText(words.running)).not.toBeNull();
        const links = (title: string) =>
            [...card(title).querySelectorAll('a')].map((link) => link.getAttribute('href'));
        expect(links(en.tools.archetypes.title)).toEqual(['/tools/duplicate-archetypes']);
        expect(links(words.heavy.title)).toEqual(['/tools/heavy-assets']);
        expect(links(words.cfg.title)).toEqual(['/cfg']);
        expect(links(words.missing.title)).toEqual(['/tools/missing-files']);
        expect(within(card(en.tools.archetypes.title)).getByText(/387 resources/)).not.toBeNull();
    });

    it('should show only the tools that need attention, or that never ran', async () => {
        const { findByText, getByRole, queryByText } = shown(<ToolsPage />, {
            'GET /servers/default/start/warnings': () => ({
                ...WARNINGS,
                startedAt: null,
                heavy: [],
                missing: [],
            }),
        });
        await findByText(words.archetypes.found(63, 4));
        await findByText(words.cfg.unheld(2));

        fireEvent.click(getByRole('button', { name: words.filters.never(2) }));
        expect([...document.querySelectorAll('article h2')].map((each) => each.textContent)).toEqual([
            words.heavy.title,
            words.missing.title,
        ]);
        fireEvent.click(getByRole('button', { name: words.filters.attention(2) }));
        expect(queryByText(words.noStart)).toBeNull();
        expect([...document.querySelectorAll('article h2')].map((each) => each.textContent)).toEqual([
            en.tools.archetypes.title,
            words.cfg.title,
        ]);
    });

    it('should show the fix that waits for the next start, and drop it', async () => {
        let staged = true;
        const { findByText, getByRole, calls } = shown(<ToolsPage />, {
            'GET /tools': () => tools(staged),
            'DELETE /tools/duplicate-archetypes/stage': () => {
                staged = false;
                return tools(false);
            },
        });

        expect(await findByText(/4 fixes of Duplicate archetypes, staged by ada/)).not.toBeNull();
        fireEvent.click(getByRole('button', { name: words.staged.drop }));

        expect(await findByText(words.staged.none)).not.toBeNull();
        expect(calls.some((call) => call.route === 'DELETE /tools/duplicate-archetypes/stage')).toBe(true);
    });

    it('should set what a tool does before each start from the hub', async () => {
        const { findByText, getByRole, calls } = shown(<ToolsPage />, {
            'PUT /tools/duplicate-archetypes/settings': () => tools(false),
        });
        await findByText(words.archetypes.found(63, 4));

        fireEvent.click(getByRole('button', { name: words.beforeStart.modes.report }));

        await waitFor(() =>
            expect(
                calls.find((call) => call.route === 'PUT /tools/duplicate-archetypes/settings')?.body,
            ).toEqual({
                settings: { atStart: 'report', differences: 'identical', escrowed: 'skip', excluded: [] },
                version: 2,
            }),
        );
    });
});

/** The rows of the list on screen, cell by cell. */
const rows = () =>
    [...document.querySelectorAll('tbody tr')].map((row) =>
        [...row.querySelectorAll('td')].map((cell) => cell.textContent),
    );

describe('ToolWarningsPage', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should list the heavy files heaviest first, and the oversized ones alone on demand', async () => {
        const { findByRole, getByRole } = shown(<ToolWarningsPage kind="heavy" />);

        expect((await findByRole('heading', { level: 2 })).textContent).toBe(words.heavy.count(3));
        expect(rows()).toEqual([
            ['maps', 'dock.ydr', '49.5 MiB', `${words.heavy.memory.physical}${words.heavy.oversized}`],
            ['cars', 'gt.ytd', '48.0 MiB', words.heavy.memory.virtual],
            ['maps', 'pier.ytd', '16.2 MiB', words.heavy.memory.physical],
        ]);
        fireEvent.click(getByRole('switch'));
        expect(rows().map((row) => row[1])).toEqual(['dock.ydr']);
        fireEvent.change(getByRole('searchbox'), { target: { value: 'cars' } });
        expect(rows()).toEqual([]);
    });

    it('should list the missing files by resource, with the manifest line that names each', async () => {
        const { findByRole } = shown(<ToolWarningsPage kind="missing" />);

        expect((await findByRole('heading', { level: 2 })).textContent).toBe(words.missing.count(2));
        expect(rows()).toEqual([
            ['audio', 'mix.rel', 'file', '__resource.lua:3'],
            ['props', 'cl.lua', 'client_script', 'fxmanifest.lua:12'],
        ]);
    });

    it('should say that nothing was seen before FXServer was started once', async () => {
        const { findByText } = shown(<ToolWarningsPage kind="missing" />, {
            'GET /servers/default/start/warnings': () => ({
                startedAt: null,
                heavy: [],
                missing: [],
                truncated: false,
            }),
        });

        expect(await findByText(words.noStartLong)).not.toBeNull();
    });
});
