import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { ArchetypeGroup } from '../api/generated/ArchetypeGroup';
import type { ArchetypeReport } from '../api/generated/ArchetypeReport';
import type { ServerState } from '../api/generated/ServerState';
import type { ToolChange } from '../api/generated/ToolChange';
import type { ToolRun } from '../api/generated/ToolRun';
import type { ToolsView } from '../api/generated/ToolsView';
import type { ToolView } from '../api/generated/ToolView';
import { ReauthDialog } from '../auth/ReauthDialog';
import { en } from '../i18n/en';
import { formatBytes, formatTimestamp } from '../i18n/format';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { ServerStatusProvider } from '../server/context';
import { ArchetypesPage } from './ArchetypesPage';

// The page links to its settings; no router runs here.
vi.mock('@tanstack/react-router', () => ({
    Link: ({
        to,
        params,
        hash,
        children,
        className,
    }: {
        to: string;
        params?: Record<string, string>;
        hash?: string;
        children: ReactNode;
        className?: string;
    }) => {
        const path = to.replace(/\$(\w+)/g, (_, name: string) => params?.[name] ?? '');
        return (
            <a href={hash === undefined ? path : `${path}#${hash}`} className={className}>
                {children}
            </a>
        );
    },
}));

const words = en.tools;
const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const PLAN = 'a1'.repeat(32);
const FRESH_PLAN = 'b2'.repeat(32);
const CAMP = 'resources/[props]/bzzz_camp/stream/bzzz_camp_props.ytyp';

/** Two identical entries of one file: fixed by taking out the first. */
const TENT: ArchetypeGroup = {
    hash: 0x2b1d_9e04,
    name: 'bzzz_camp_tent',
    copies: [
        {
            resource: 'bzzz_camp',
            path: CAMP,
            source: { format: 'ytyp', index: 0, archetype: 'base' },
            escrowed: false,
        },
        {
            resource: 'bzzz_camp',
            path: CAMP,
            source: { format: 'ytyp', index: 3, archetype: 'base' },
            escrowed: false,
        },
    ],
    difference: { kind: 'identical' },
    sameFile: true,
    action: { kind: 'fix', keep: 1, remove: [0] },
};

/** Copies in two resources, the second one's name contested. */
const GIFT: ArchetypeGroup = {
    hash: 0xf032_591f,
    name: 'bz_prop_gift',
    copies: [
        {
            resource: 'bz_gifts',
            path: 'resources/bz_gifts/stream/bz_prop_gift.ytyp',
            source: { format: 'ytyp', index: 0, archetype: 'base' },
            escrowed: false,
        },
        {
            resource: 'bz_gifts2',
            path: 'resources/bz_gifts2/stream/bz_prop_gift2.ytyp',
            source: { format: 'ytyp', index: 0, archetype: 'base' },
            escrowed: true,
        },
    ],
    difference: { kind: 'minor', fields: ['lodDist'] },
    sameFile: false,
    action: {
        kind: 'leave',
        reasons: [
            { kind: 'severalResources' },
            {
                kind: 'contested',
                file: 'resources/bz_gifts2/stream/bz_prop_gift2.ytyp',
                by: {
                    kind: 'sameName',
                    resource: 'old_gifts',
                    path: 'resources/old_gifts/stream/bz_prop_gift2.ytyp',
                },
            },
        ],
    },
};

/** Two weapon items of one meta that differ: left, its name unknown. */
const ITEM: ArchetypeGroup = {
    hash: 0x0000_beef,
    name: null,
    copies: [
        {
            resource: 'weapons',
            path: 'resources/weapons/meta/weapons.meta',
            source: { format: 'modelInfo', kind: 'weapon', ordinal: 1, line: 40 },
            escrowed: false,
        },
        {
            resource: 'weapons',
            path: 'resources/weapons/meta/weapons.meta',
            source: { format: 'modelInfo', kind: 'weapon', ordinal: 4, line: 112 },
            escrowed: false,
        },
    ],
    difference: { kind: 'major', fields: ['flags'] },
    sameFile: true,
    action: { kind: 'leave', reasons: [{ kind: 'definitionsDiffer' }] },
};

const reportOf = (groups: ArchetypeGroup[]): ArchetypeReport => ({
    folder: 'D:\\FXServer\\server-data',
    counts: {
        resources: 376,
        started: 340,
        client: 290,
        escrowed: 120,
        ytypLoaded: 665,
        ytypShadowed: 1,
        ytypUnreadable: 1,
        modelInfoLoaded: 779,
        modelInfoUnreadable: 0,
        registrations: 21_485,
        groups: groups.length,
        groupsFixed: groups.filter((group) => group.action.kind === 'fix').length,
        filesChanged: groups.some((group) => group.action.kind === 'fix') ? 1 : 0,
        removals: groups.filter((group) => group.action.kind === 'fix').length,
    },
    groups,
    groupsTotal: groups.length,
    shadowed: [
        {
            resource: 'old_gifts',
            path: 'resources/old_gifts/stream/bz_prop_gift2.ytyp',
            ownerResource: 'bz_gifts2',
            ownerPath: 'resources/bz_gifts2/stream/bz_prop_gift2.ytyp',
        },
    ],
    shadowedTotal: 1,
    unreadable: [
        {
            resource: 'bz_gifts2',
            path: 'resources/bz_gifts2/stream/locked.ytyp',
            problem: { kind: 'format', found: 'escrowed' },
        },
    ],
    unreadableTotal: 1,
    notes: [{ kind: 'baseGameUnseen', resource: null, file: null, line: null, detail: null, count: 1 }],
    notesTotal: 1,
    timings: { walkMs: 900, parseMs: 1200, totalMs: 2140 },
});

const runOf = (run: Partial<ToolRun> & Pick<ToolRun, 'id'>): ToolRun => ({
    tool: 'duplicateArchetypes',
    trigger: 'panel',
    actor: 'ada',
    mode: 'report',
    startedAt: '2026-11-01T09:00:00Z',
    finishedAt: '2026-11-01T09:00:02Z',
    outcome: 'done',
    summary: {
        groups: 3,
        fixable: 1,
        fixed: 0,
        left: 2,
        filesChanged: 0,
        filesRestored: 0,
        removals: 0,
        openChanges: 2,
        elapsedMs: 2140,
        problem: null,
    },
    plan: PLAN,
    report: reportOf([TENT, GIFT, ITEM]),
    ...run,
});

const changeOf = (id: number, path: string, run: number): ToolChange => ({
    id,
    folder: 'D:\\FXServer\\server-data',
    path,
    resource: path.split('/')[1] ?? '',
    format: 'ytyp',
    removals: [
        {
            index: 2,
            hash: 0x1234_abcd,
            name: `prop_${id}`,
            keeper: { path, format: 'ytyp', index: 5, sha256: 'c3'.repeat(32) },
            run,
        },
    ],
    state: 'applied',
    closeReason: null,
    backup: 'd4'.repeat(32),
    openedAt: '2026-10-30T10:00:00Z',
    changedAt: '2026-10-30T10:00:01Z',
    closedAt: null,
});

/** The change a fix run made to CAMP: the first copy of TENT taken out. */
const tentOut = (id: number, run: number): ToolChange => ({
    ...changeOf(id, CAMP, run),
    removals: [
        {
            index: 0,
            hash: TENT.hash,
            name: TENT.name,
            keeper: { path: CAMP, format: 'ytyp', index: 3, sha256: 'c3'.repeat(32) },
            run,
        },
    ],
    changedAt: '2026-11-01T09:00:01Z',
});

const toolOf = (tool: Partial<ToolView> = {}): ToolView => ({
    tool: 'duplicateArchetypes',
    settings: { atStart: 'off', differences: 'identical', escrowed: 'skip', excluded: [] },
    version: 2,
    lastRun: runOf({ id: 7 }),
    lastReport: null,
    open: [
        changeOf(11, 'resources/props_a/stream/a.ytyp', 3),
        changeOf(12, 'resources/props_b/stream/b.ytyp', 5),
    ],
    elsewhere: [],
    unreachable: [],
    closed: [
        {
            ...changeOf(9, 'resources/props_c/stream/c.ytyp', 2),
            state: 'closed',
            closeReason: 'undone',
            closedAt: '2026-10-31T08:00:00Z',
        },
    ],
    backupBytes: 2_400_000,
    staged: null,
    ...tool,
});

const viewOf = (tool: Partial<ToolView> = {}): ToolsView => ({ tools: [toolOf(tool)] });

const statusOf = (state: ServerState) => ({
    state,
    cause: null,
    since: '2026-11-01T08:00:00Z',
    startedAt: null,
    generation: 1,
    problems: [],
    build: null,
    players: null,
    restartAt: null,
    launch: 'ready',
    start: null,
    stopHolders: [],
    stop: null,
});

const answer = (status: number, body?: unknown) =>
    new Response(body === undefined ? null : JSON.stringify(body), {
        status,
        headers: { 'content-type': 'application/json' },
    });

type Reply = unknown | ((body: unknown) => unknown);

/** Observer's answers by method and route; every request made is kept, with its body. */
const serve = (replies: Record<string, Reply>) => {
    const asked: { route: string; body: unknown }[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string, init: RequestInit) => {
            const route = `${init.method ?? 'GET'} ${new URL(input, 'http://localhost').pathname.replace(/^\/api/, '')}`;
            const body = typeof init.body === 'string' ? JSON.parse(init.body) : undefined;
            asked.push({ route, body });
            const found = replies[route];
            const reply = typeof found === 'function' ? (found as (body: unknown) => unknown)(body) : found;
            return reply instanceof Response
                ? reply.clone()
                : answer(reply === undefined ? 404 : 200, reply ?? { code: 'not_found', params: {} });
        }),
    );
    return asked;
};

const links: Realtime[] = [];

/** The page, FXServer in `state` as the status topic says. */
const page = async (state: ServerState) => {
    let socket: SocketEvents | undefined;
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: () => {}, close: () => {} };
        },
        refetch: () => {},
    });
    links.push(live);
    const view = render(
        <QueryClientProvider
            client={
                new QueryClient({
                    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
                })
            }
        >
            <I18nProvider>
                <LiveContext value={live}>
                    <ServerStatusProvider>
                        <ArchetypesPage />
                        <ReauthDialog />
                    </ServerStatusProvider>
                </LiveContext>
            </I18nProvider>
        </QueryClientProvider>,
    );
    live.start();
    await settle();
    const say = (message: unknown) => act(() => socket?.message(JSON.stringify(message)));
    say({ type: 'hello', connectionId: 1, contract: 1 });
    say({ type: 'snapshot', topic: 'server.status', data: statusOf(state) });
    const becomes = (next: ServerState) =>
        say({ type: 'event', topic: 'server.status', data: statusOf(next) });
    await view.findByText(words.archetypes.about);
    return { ...view, becomes };
};

const button = (name: string) => (container: ReturnType<typeof within>) =>
    container.getByRole('button', { name }) as HTMLButtonElement;

describe('ArchetypesPage', () => {
    beforeAll(() => {
        // jsdom has the element but not its modal methods.
        HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
            this.open = true;
        };
        HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
            this.open = false;
        };
    });

    afterEach(() => {
        for (const live of links.splice(0)) {
            live.stop();
        }
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should explain the tool, then show the last run, its counts and what the report found', async () => {
        serve({ 'GET /tools': viewOf() });
        const { getByText, getByRole, getAllByText } = await page('stopped');

        const lastRun = getByRole('region', { name: words.archetypes.lastRun });
        expect(within(lastRun).getByText(words.archetypes.byActor('ada'))).not.toBeNull();
        expect(within(lastRun).getByText(words.archetypes.modes.report)).not.toBeNull();
        expect(within(lastRun).getByText(words.archetypes.outcomes.done)).not.toBeNull();
        expect(within(lastRun).getByText(words.archetypes.summary.openChanges).nextSibling?.textContent).toBe(
            '2',
        );
        // Counts at zero for a report stay out of the way.
        expect(within(lastRun).queryByText(words.archetypes.summary.filesChanged)).toBeNull();

        expect(getByText(words.archetypes.fixPlan(1, 1, 1))).not.toBeNull();
        expect(getByText('bzzz_camp_tent')).not.toBeNull();
        expect(getByText(words.groups.keeps(2, [1]))).not.toBeNull();
        expect(getByText('0000BEEF')).not.toBeNull();
        expect(getByText(words.groups.reason({ kind: 'severalResources' }))).not.toBeNull();
        expect(
            getByText(
                'resources/bz_gifts2/stream/bz_prop_gift2.ytyp: resources/old_gifts/stream/bz_prop_gift2.ytyp in old_gifts has the same name and may be loaded in its place.',
            ),
        ).not.toBeNull();
        expect(
            getByText(`resources/weapons/meta/weapons.meta · ${words.groups.item('weapon', 4, 112)}`),
        ).not.toBeNull();
        expect(getAllByText(words.groups.differences.identical)).toHaveLength(1);
        expect(getByText('flags')).not.toBeNull();

        fireEvent.click(getByRole('button', { name: words.report.unreadable(1) }));
        expect(getByText('Encrypted by the asset escrow.')).not.toBeNull();
        fireEvent.click(getByRole('button', { name: words.report.notes(1) }));
        expect(getByText(words.report.noteKinds.baseGameUnseen)).not.toBeNull();
    });

    it('should narrow the groups to an action, a reason and a search', async () => {
        serve({ 'GET /tools': viewOf() });
        const { getByRole, getByLabelText, queryByText, getByText } = await page('stopped');
        const table = () => getByRole('table', { name: words.groups.caption });
        const names = () =>
            within(table())
                .getAllByRole('row')
                .slice(1)
                .map((row) => row.querySelector('td')?.firstElementChild?.textContent);

        expect(names()).toEqual(['bzzz_camp_tent', 'bz_prop_gift', '0000BEEF']);

        fireEvent.click(getByRole('button', { name: `${words.groups.actions.left} 2` }));
        expect(names()).toEqual(['bz_prop_gift', '0000BEEF']);

        fireEvent.click(getByRole('button', { name: `${words.groups.reasons.definitionsDiffer} 1` }));
        expect(names()).toEqual(['0000BEEF']);

        fireEvent.click(getByRole('button', { name: `${words.groups.reasons.definitionsDiffer} 1` }));
        fireEvent.change(getByLabelText(words.groups.search), { target: { value: 'f032' } });
        expect(names()).toEqual(['bz_prop_gift']);

        fireEvent.change(getByLabelText(words.groups.search), { target: { value: 'nothing like it' } });
        expect(queryByText(words.groups.caption)).toBeNull();
        expect(getByText(words.groups.noMatch)).not.toBeNull();
    });

    it('should fix only while FXServer is stopped, naming the plan on screen', async () => {
        const fixed = runOf({
            id: 8,
            mode: 'fix',
            summary: { ...runOf({ id: 8 }).summary!, fixed: 1, filesChanged: 1, removals: 1 },
        });
        let current = viewOf();
        const asked = serve({
            'GET /tools': () => current,
            'POST /tools/duplicate-archetypes/fix': () => {
                current = viewOf({ lastRun: fixed, open: [...viewOf().tools[0]!.open, tentOut(13, 8)] });
                return fixed;
            },
        });
        const { getByRole, getByText, findByText, becomes } = await page('online');
        const card = getByRole('region', { name: words.archetypes.title });
        const fix = () => button(words.archetypes.fix)(within(card));

        // While FXServer runs, the fix can only be staged for its next start.
        expect(within(card).queryByRole('button', { name: words.archetypes.fix })).toBeNull();
        expect(within(card).getByRole('button', { name: words.archetypes.stage(1) })).not.toBeNull();
        expect(within(card).getByText(words.archetypes.stageWaits(1))).not.toBeNull();

        becomes('stopped');
        expect(fix().disabled).toBe(false);
        expect(within(card).queryByText(words.archetypes.stageWaits(1))).toBeNull();
        expect(within(card).getByText(words.archetypes.fixPlan(1, 1, 1))).not.toBeNull();
        fireEvent.click(fix());

        expect(await findByText(words.archetypes.fixed(1, 1))).not.toBeNull();
        expect(
            asked.find((request) => request.route === 'POST /tools/duplicate-archetypes/fix')?.body,
        ).toEqual({
            plan: PLAN,
        });
        // The report now says what was fixed: fixing it again would change nothing.
        await waitFor(() => expect(getByText(words.archetypes.applied)).not.toBeNull());
        expect(fix().disabled).toBe(true);
        expect(getByRole('button', { name: `${words.groups.actions.fixed} 1` })).not.toBeNull();
    });

    it('should stage the plan on screen for the next start while FXServer runs, and drop it again', async () => {
        const staged = {
            plan: PLAN,
            fixes: 1,
            stagedAt: '2026-11-01T09:05:00Z',
            stagedBy: 'ada',
        };
        let current = viewOf();
        const asked = serve({
            'GET /tools': () => current,
            'POST /tools/duplicate-archetypes/stage': () => {
                current = viewOf({ staged });
                return current;
            },
            'DELETE /tools/duplicate-archetypes/stage': () => {
                current = viewOf();
                return current;
            },
        });
        const { getByRole, findByText, queryByText } = await page('online');
        const card = getByRole('region', { name: words.archetypes.title });

        fireEvent.click(within(card).getByRole('button', { name: words.archetypes.stage(1) }));

        expect(await findByText(words.archetypes.stagedBy(1, 'ada'))).not.toBeNull();
        expect(
            asked.find((request) => request.route === 'POST /tools/duplicate-archetypes/stage')?.body,
        ).toEqual({ plan: PLAN });
        // Staged, there is nothing more to stage: the fix shows again, for a server that stops.
        expect(within(card).queryByRole('button', { name: words.archetypes.stage(1) })).toBeNull();
        expect(button(words.archetypes.fix)(within(card)).disabled).toBe(true);
        fireEvent.click(within(card).getByRole('button', { name: words.archetypes.unstage }));

        await waitFor(() => expect(queryByText(words.archetypes.stagedBy(1, 'ada'))).toBeNull());
        expect(within(card).getByRole('button', { name: words.archetypes.stage(1) })).not.toBeNull();
    });

    it('should show the fresh report with a notice when the plan changed since the one on screen', async () => {
        const fresh = runOf({
            id: 9,
            mode: 'fix',
            plan: FRESH_PLAN,
            summary: {
                ...runOf({ id: 9 }).summary!,
                groups: 1,
                fixable: 0,
                left: 1,
                problem: 'the plan changed',
            },
            report: reportOf([ITEM]),
        });
        let current = viewOf();
        const asked = serve({
            'GET /tools': () => current,
            'POST /tools/duplicate-archetypes/fix': () => {
                current = viewOf({ lastRun: fresh });
                return answer(409, { code: 'plan_changed', params: { run: fresh } });
            },
        });
        const { getByRole, findByText, queryByText, getByText } = await page('stopped');

        fireEvent.click(
            button(words.archetypes.fix)(within(getByRole('region', { name: words.archetypes.title }))),
        );

        expect(await findByText(en.errors.plan_changed)).not.toBeNull();
        expect(queryByText('bzzz_camp_tent')).toBeNull();
        expect(getByText('0000BEEF')).not.toBeNull();
        expect(getByText(words.archetypes.nothingToFix)).not.toBeNull();
        expect(
            asked.filter((request) => request.route === 'POST /tools/duplicate-archetypes/fix'),
        ).toHaveLength(1);
    });

    it('should ask for the password again before fixing, then fix', async () => {
        const fixed = runOf({
            id: 8,
            mode: 'fix',
            summary: { ...runOf({ id: 8 }).summary!, fixed: 1, filesChanged: 1, removals: 1 },
        });
        const replies = [
            answer(403, { code: 'reauth_required', params: {} }),
            answer(204),
            answer(200, fixed),
        ];
        const asked = serve({
            'GET /tools': viewOf(),
            'POST /tools/duplicate-archetypes/fix': () => replies.shift(),
            'POST /auth/reauth': () => replies.shift(),
        });
        const { getByRole, findByLabelText, findByText } = await page('stopped');

        fireEvent.click(
            button(words.archetypes.fix)(within(getByRole('region', { name: words.archetypes.title }))),
        );
        fireEvent.change(await findByLabelText(en.reauth.password), {
            target: { value: 'correct horse battery staple' },
        });
        fireEvent.click(getByRole('button', { name: en.reauth.submit }));

        expect(await findByText(words.archetypes.fixed(1, 1))).not.toBeNull();
        expect(asked.map((request) => request.route).filter((route) => route.startsWith('POST'))).toEqual([
            'POST /tools/duplicate-archetypes/fix',
            'POST /auth/reauth',
            'POST /tools/duplicate-archetypes/fix',
        ]);
    });

    it('should undo a file, a run or everything, only while FXServer is stopped', async () => {
        const report = runOf({ id: 7 });
        const undone = runOf({
            id: 10,
            mode: 'undo',
            plan: null,
            report: null,
            summary: { ...report.summary!, filesRestored: 1 },
        });
        let current = viewOf();
        const asked = serve({
            'GET /tools': () => current,
            'POST /tools/duplicate-archetypes/undo': () => {
                current = viewOf({ lastRun: undone, lastReport: report });
                return undone;
            },
        });
        const { getByRole, findByText, becomes, getByText } = await page('online');
        const card = getByRole('region', { name: words.changes.title });
        const undos = () =>
            asked
                .filter((request) => request.route === 'POST /tools/duplicate-archetypes/undo')
                .map((request) => request.body);

        expect(button(words.changes.undoAll)(within(card)).disabled).toBe(true);
        expect(within(card).getByText(words.stopFirst)).not.toBeNull();
        expect(within(card).getByText(words.changes.run(5, 1, 1))).not.toBeNull();
        expect(within(card).getByText(words.changes.closeReasons.undone)).not.toBeNull();
        expect(within(card).getByText(words.changes.backups(formatBytes('en', 2_400_000)))).not.toBeNull();

        becomes('stopped');
        fireEvent.click(button(words.changes.undoFile('resources/props_a/stream/a.ytyp'))(within(card)));
        expect(await findByText(words.changes.undone(1))).not.toBeNull();
        await waitFor(() => expect(button(words.changes.undoRun(5))(within(card)).disabled).toBe(false));
        fireEvent.click(button(words.changes.undoRun(5))(within(card)));
        await waitFor(() => expect(undos()).toHaveLength(2));
        await waitFor(() => expect(button(words.changes.undoAll)(within(card)).disabled).toBe(false));
        fireEvent.click(button(words.changes.undoAll)(within(card)));
        await waitFor(() => expect(undos()).toHaveLength(3));

        expect(undos()).toEqual([{ scope: { changes: [11] } }, { scope: { run: 5 } }, { scope: 'all' }]);
        // An undo makes no report: the groups stay those of the report before, which says so.
        expect(getByText('bzzz_camp_tent')).not.toBeNull();
        expect(
            getByText(words.archetypes.reportFrom(formatTimestamp(new Date(report.startedAt)))),
        ).not.toBeNull();
    });

    it('should say the report is out of date once an undo put entries back, and drop what the scan said', async () => {
        const scanned = runOf({
            id: 8,
            summary: { ...runOf({ id: 8 }).summary!, groups: 2, fixable: 0, left: 2 },
            report: reportOf([GIFT, ITEM]),
        });
        const undone = runOf({
            id: 9,
            mode: 'undo',
            startedAt: '2026-11-01T09:05:00Z',
            finishedAt: '2026-11-01T09:05:01Z',
            plan: null,
            report: null,
            summary: { ...runOf({ id: 9 }).summary!, filesRestored: 1, openChanges: 0 },
        });
        let current = viewOf({ lastRun: runOf({ id: 7 }), open: [tentOut(13, 6)] });
        serve({
            'GET /tools': () => current,
            'POST /tools/duplicate-archetypes/scan': () => {
                current = viewOf({ lastRun: scanned, open: [tentOut(13, 6)] });
                return scanned;
            },
            'POST /tools/duplicate-archetypes/undo': () => {
                current = viewOf({
                    lastRun: undone,
                    lastReport: scanned,
                    open: [],
                    closed: [
                        {
                            ...tentOut(13, 6),
                            state: 'closed',
                            closeReason: 'undone',
                            closedAt: '2026-11-01T09:05:01Z',
                        },
                    ],
                });
                return undone;
            },
        });
        const { getByRole, findByText, getByText, queryByText } = await page('stopped');
        const card = getByRole('region', { name: words.archetypes.title });

        fireEvent.click(button(words.archetypes.scan)(within(card)));
        expect(await findByText(words.archetypes.scanned(2, 0))).not.toBeNull();
        expect(getByText(words.archetypes.nothingToFix)).not.toBeNull();

        fireEvent.click(
            button(words.changes.undoAll)(within(getByRole('region', { name: words.changes.title }))),
        );

        expect(await findByText(words.changes.undone(1))).not.toBeNull();
        await waitFor(() => expect(getByText(words.archetypes.outdated)).not.toBeNull());
        expect(queryByText(words.archetypes.scanned(2, 0))).toBeNull();
        expect(queryByText(words.archetypes.nothingToFix)).toBeNull();
        expect(button(words.archetypes.fix)(within(card)).disabled).toBe(true);
    });

    it('should show groups as fixed only while the files still hold their fix', async () => {
        const fixed = runOf({
            id: 8,
            mode: 'fix',
            report: reportOf([TENT, GIFT, ITEM]),
            summary: { ...runOf({ id: 8 }).summary!, fixed: 1, filesChanged: 1, removals: 1 },
        });
        // Its file was put back by hand: the change closed without an undo of Observer's.
        serve({ 'GET /tools': viewOf({ lastRun: fixed }) });
        const { getByRole, getByText, queryByText } = await page('stopped');
        const card = getByRole('region', { name: words.archetypes.title });
        const tent = within(getByRole('table', { name: words.groups.caption })).getAllByRole('row')[1]!;

        expect(within(tent).getByText(words.groups.actions.toFix)).not.toBeNull();
        expect(within(tent).queryByText(words.groups.actions.fixed)).toBeNull();
        expect(queryByText(words.archetypes.applied)).toBeNull();
        expect(getByText(words.archetypes.fixPlan(1, 1, 1))).not.toBeNull();
        expect(button(words.archetypes.fix)(within(card)).disabled).toBe(false);
    });

    it('should count no duplicates for a run that analysed nothing', async () => {
        const undone = runOf({
            id: 10,
            mode: 'undo',
            plan: null,
            report: null,
            summary: { ...runOf({ id: 10 }).summary!, groups: 0, fixable: 0, left: 0, filesRestored: 4 },
        });
        serve({ 'GET /tools': viewOf({ lastRun: undone, lastReport: runOf({ id: 7 }) }) });
        const { getByRole } = await page('stopped');

        const lastRun = getByRole('region', { name: words.archetypes.lastRun });
        expect(within(lastRun).queryByText(words.archetypes.summary.groups)).toBeNull();
        expect(within(lastRun).queryByText(words.archetypes.summary.fixable)).toBeNull();
        expect(within(lastRun).queryByText(words.archetypes.summary.left)).toBeNull();
        expect(
            within(lastRun).getByText(words.archetypes.summary.filesRestored).nextSibling?.textContent,
        ).toBe('4');
        expect(within(lastRun).getByText(words.archetypes.summary.openChanges)).not.toBeNull();
    });

    it('should read the page again as FXServer starts and stops, the tool having run before the start', async () => {
        const asked = serve({ 'GET /tools': viewOf() });
        const { becomes } = await page('stopped');
        const reads = () => asked.filter((request) => request.route === 'GET /tools').length;
        const before = reads();

        becomes('starting');
        await waitFor(() => expect(reads()).toBe(before + 1));
        becomes('online');
        await waitFor(() => expect(reads()).toBe(before + 2));
        becomes('online');
        await settle();
        expect(reads()).toBe(before + 2);
    });

    it('should read the page again when an undo is refused, its change gone', async () => {
        const asked = serve({ 'GET /tools': viewOf() });
        const { getByRole, findByText } = await page('stopped');
        const reads = () => asked.filter((request) => request.route === 'GET /tools').length;
        const before = reads();

        fireEvent.click(
            button(words.changes.undoFile('resources/props_a/stream/a.ytyp'))(
                within(getByRole('region', { name: words.changes.title })),
            ),
        );

        expect(await findByText(en.errors.not_found)).not.toBeNull();
        await waitFor(() => expect(reads()).toBe(before + 1));
    });

    it('should undo the changes of another folder by file or by run, and none whose folder is gone', async () => {
        const asked = serve({
            'GET /tools': viewOf({
                open: [
                    changeOf(11, 'resources/props_a/stream/a.ytyp', 3),
                    { ...changeOf(12, 'resources/props_b/stream/b.ytyp', 5), folder: 'E:\\copy' },
                    { ...changeOf(14, 'resources/props_d/stream/d.ytyp', 6), folder: 'F:\\gone' },
                ],
                elsewhere: [12, 14],
                unreachable: [14],
            }),
            'POST /tools/duplicate-archetypes/undo': runOf({
                id: 20,
                mode: 'undo',
                plan: null,
                report: null,
            }),
        });
        const { getByRole, getByText } = await page('stopped');
        const card = within(getByRole('region', { name: words.changes.title }));

        expect(getByText(words.changes.elsewhere)).not.toBeNull();
        expect(card.getByText('E:\\copy')).not.toBeNull();
        const gone = button(words.changes.undoFile('resources/props_d/stream/d.ytyp'))(card);
        expect(gone.disabled).toBe(true);
        expect(document.getElementById(gone.getAttribute('aria-describedby') ?? '')?.textContent).toBe(
            words.changes.unreachable,
        );
        expect(card.queryByRole('button', { name: words.changes.undoRun(6) })).toBeNull();
        expect(button(words.changes.undoAll)(card).disabled).toBe(false);

        fireEvent.click(button(words.changes.undoRun(5))(card));
        await waitFor(() =>
            expect(
                asked
                    .filter((request) => request.route === 'POST /tools/duplicate-archetypes/undo')
                    .map((request) => request.body),
            ).toEqual([{ scope: { run: 5 } }]),
        );
    });

    it('should say there is nothing to fix before the first scan, and scan on demand', async () => {
        const scanned = runOf({ id: 1 });
        let current = viewOf({ lastRun: null, open: [], closed: [], backupBytes: 0 });
        serve({
            'GET /tools': () => current,
            'POST /tools/duplicate-archetypes/scan': () => {
                current = viewOf({ lastRun: scanned, open: [], closed: [], backupBytes: 0 });
                return scanned;
            },
        });
        const { getByRole, getByText, findByText } = await page('online');
        const card = getByRole('region', { name: words.archetypes.title });

        expect(getByText(words.archetypes.noRun)).not.toBeNull();
        expect(getByText(words.groups.noReport)).not.toBeNull();
        expect(getByText(words.changes.none)).not.toBeNull();
        expect(button(words.archetypes.fix)(within(card)).disabled).toBe(true);
        expect(getByText(words.archetypes.noReport)).not.toBeNull();

        fireEvent.click(button(words.archetypes.scan)(within(card)));

        expect(await findByText(words.archetypes.scanned(3, 1))).not.toBeNull();
        expect(getByText('bzzz_camp_tent')).not.toBeNull();
        // A scan works while FXServer runs; the fix it plans is staged, or waits for it to stop.
        expect(within(card).getByRole('button', { name: words.archetypes.stage(1) })).not.toBeNull();
        expect(within(card).getByText(words.archetypes.stageWaits(1))).not.toBeNull();
        // The two it leaves are counted by reason, with what decides of them.
        expect(getByRole('heading', { name: words.archetypes.left.title(2) })).not.toBeNull();
    });
});
