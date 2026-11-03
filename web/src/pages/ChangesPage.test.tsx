import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
    Outlet,
    RouterProvider,
} from '@tanstack/react-router';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ChangesLive } from '../api/generated/ChangesLive';
import type { Comparison } from '../api/generated/Comparison';
import type { FileDiff } from '../api/generated/FileDiff';
import type { FileText } from '../api/generated/FileText';
import type { Me } from '../api/generated/Me';
import type { ResourceChange } from '../api/generated/ResourceChange';
import type { SnapshotSummary } from '../api/generated/SnapshotSummary';
import { MeContext } from '../auth/me';
import { ReauthDialog } from '../auth/ReauthDialog';
import {
    BASELINE,
    CLIENT,
    COMPARISON,
    CONFIG,
    CONFIG_DIFF,
    DETAIL,
    INVENTORY,
    LATEST,
    RESTART,
    SETTINGS,
    STARTS,
} from '../changes/fixtures';
import { REFRESH_MS } from '../changes/live';
import { changesSearch, resourceSearch } from '../changes/search';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { ChangesPage } from './ChangesPage';
import { ChangesResourcePage } from './ChangesResourcePage';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

// Written by code point: a source file holding it would hide it from its own review.
const RLO = String.fromCodePoint(0x202e);

const CHECK: SnapshotSummary = {
    ...LATEST,
    id: 9,
    kind: 'check',
    takenAt: '2026-10-31T16:00:00Z',
    cause: null,
    by: null,
    counts: null,
};

// Bit 0 the dashboard, 16 the changes, 17 their contents.
const OWNER: Me = { id: 1, username: 'ada', isOwner: true, permissions: '7ffff', mustChangePassword: false };
const VIEWER: Me = {
    id: 2,
    username: 'kenji',
    isOwner: false,
    permissions: '20001',
    mustChangePassword: false,
};
const READER: Me = {
    id: 3,
    username: 'bob',
    isOwner: false,
    permissions: '60001',
    mustChangePassword: false,
};

const answer = (status: number, body?: unknown) =>
    new Response(body === undefined ? null : JSON.stringify(body), {
        status,
        headers: { 'content-type': 'application/json' },
    });

/**
 * Observer's answers by method and route, or a function of the request's query; every request made
 * is kept, with its query.
 */
const serve = (replies: Record<string, unknown>) => {
    const asked: string[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string, init: RequestInit) => {
            const url = new URL(input, 'http://localhost');
            const route = `${init.method ?? 'GET'} ${url.pathname.replace(/^\/api/, '')}`;
            asked.push(`${route}${url.search}`);
            const found = replies[route];
            const reply =
                typeof found === 'function'
                    ? (found as (query: URLSearchParams) => unknown)(url.searchParams)
                    : found;
            return reply instanceof Response
                ? reply.clone()
                : answer(reply === undefined ? 404 : 200, reply ?? { code: 'not_found' });
        }),
    );
    return asked;
};

const SERVED = {
    'GET /servers/default/changes/starts': STARTS,
    'GET /servers/default/changes': COMPARISON,
    'GET /servers/default/changes/settings': SETTINGS,
    'GET /servers/default/changes/resource': DETAIL,
    'GET /servers/default/changes/history': [],
    'GET /servers/default/changes/diff': CONFIG_DIFF,
};

/** Realtime links the tests started, stopped after each. */
const links: Realtime[] = [];

/** The Changes pages at `path` for `me`; `server.changes` says what the test says. */
const panel = async (path: string, me: Me) => {
    const root = createRootRoute({ component: Outlet });
    const changes = createRoute({
        getParentRoute: () => root,
        path: '/changes',
        validateSearch: changesSearch,
        component: ChangesPage,
    });
    const resource = createRoute({
        getParentRoute: () => root,
        path: '/changes/resource',
        validateSearch: resourceSearch,
        component: ChangesResourcePage,
    });
    const router = createRouter({
        routeTree: root.addChildren([changes, resource]),
        history: createMemoryHistory({ initialEntries: [path] }),
    });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
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
        <I18nProvider>
            <QueryClientProvider client={client}>
                <MeContext value={me}>
                    <LiveContext value={live}>
                        <RouterProvider router={router} />
                        <ReauthDialog />
                    </LiveContext>
                </MeContext>
            </QueryClientProvider>
        </I18nProvider>,
    );
    live.start();
    await settle();
    await settle();
    act(() => socket?.message(JSON.stringify({ type: 'hello', connectionId: 1, contract: 1 })));
    const say = (type: 'snapshot' | 'event', data: ChangesLive) =>
        act(() => socket?.message(JSON.stringify({ type, topic: 'server.changes', data })));
    return { ...view, router, client, say };
};

/** A resource of the comparison, as the API lists it. */
const changeOf = (change: Partial<ResourceChange> & Pick<ResourceChange, 'name' | 'change'>) => ({
    ...INVENTORY,
    path: `resources/[qb]/${change.name}`,
    fromVersion: null,
    toVersion: null,
    escrowed: false,
    by: [],
    ...change,
});

/** A diff of one hunk, its lines as given. */
const diffOf = (
    file: FileDiff['file'],
    lines: { kind: 'added' | 'removed' | 'context'; text: string }[],
): FileDiff => ({
    file,
    diff: {
        added: lines.filter((line) => line.kind === 'added').length,
        removed: lines.filter((line) => line.kind === 'removed').length,
        lineEndingsOnly: false,
        fromTotal: 40,
        toTotal: 40,
        page: 0,
        pages: 1,
        hunks: [
            {
                fromStart: 11,
                fromLines: lines.length,
                toStart: 11,
                toLines: lines.length,
                lines: lines.map((line, index) => ({
                    kind: line.kind,
                    from: line.kind === 'added' ? null : 11 + index,
                    to: line.kind === 'removed' ? null : 11 + index,
                    segments: [{ text: line.text, changed: false, redacted: false }],
                })),
            },
        ],
    },
});

const card = (title: string) => {
    const heading = [...document.querySelectorAll('h2')].find((element) => element.textContent === title);
    const section = heading?.closest('section');
    if (section === null || section === undefined) {
        throw new Error(`no ${title} card`);
    }
    return within(section);
};

/** A fact of the summary band: its figure, then what it says under it. */
const fact = (label: string) => {
    const term = [...document.querySelectorAll('dt')].find((element) => element.textContent === label);
    return [...(term?.nextElementSibling?.children ?? [])].map((part) => part.textContent);
};

/** The starts of the timeline, newest first. */
const starts = () => card(en.changes.starts.title).getAllByRole('listitem');

/** The names in the resources table, in order. */
const rows = () =>
    [...document.querySelectorAll('tbody tr td:first-child a')].map((link) => link.textContent);

/** The way to the settings that sat at the bottom of the page, now in Data kept. */
const settingsLink = () =>
    [...document.querySelectorAll('a')].find((link) =>
        link.textContent?.startsWith(en.settings.moved(en.settings.categories.data.name)),
    );

describe('ChangesPage', () => {
    beforeAll(() => {
        // jsdom has the element but not its modal methods.
        HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
            this.open = true;
        };
        HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
            this.open = false;
        };
    });

    beforeEach(() => {
        // Each navigation scrolls to the top, which jsdom does not implement.
        vi.stubGlobal('scrollTo', () => {});
    });

    afterEach(() => {
        cleanup();
        for (const live of links.splice(0)) {
            live.stop();
        }
        vi.useRealTimers();
        vi.unstubAllGlobals();
        vi.restoreAllMocks();
    });

    it('should count what changed between the last two starts, the build and the cfg included', async () => {
        serve(SERVED);
        await panel('/changes', VIEWER);

        expect(fact(en.changes.summary.resources)).toEqual([
            '6',
            '2 updated · 1 same version · 1 added · 1 removed · 1 did not start',
        ]);
        expect(fact(en.changes.summary.files)).toEqual(['9', en.changes.summary.filesSub]);
        expect(fact(en.changes.summary.build)).toEqual(['26100', 'from 25770']);
        expect(fact(en.changes.summary.cfg)).toEqual(['2 files', '+5 −1']);
        expect(fact(en.changes.summary.launch)).toEqual([en.changes.summary.same]);
        const environment = card(en.changes.environment.title);
        expect(environment.getByText('25770 → 26100')).not.toBeNull();
        expect(environment.getByText(/^Build selected by ada · /)).not.toBeNull();
        expect(environment.getByText('2 files changed · +5 −1')).not.toBeNull();
        expect(environment.getByText('qb-housing')).not.toBeNull();
    });

    it('should list the resources whose files changed under the same version first, as a warning', async () => {
        serve(SERVED);
        await panel('/changes', VIEWER);

        expect(rows()).toEqual(['qb-inventory', 'qb-core', 'ox_lib', 'jg-dealerships', 'old-hud', 'chat']);
        const inventory = document.querySelector('tbody tr');
        expect(inventory?.className).toContain('bg-warning-soft');
        expect(
            within(inventory as HTMLElement).getByText(en.changes.resources.kinds.sameVersion),
        ).not.toBeNull();
        expect(within(inventory as HTMLElement).getByText('+0 ~3 −0')).not.toBeNull();
        expect(within(inventory as HTMLElement).getByText(en.changes.resources.escrowed)).not.toBeNull();
        expect(card(en.changes.resources.title).getByText('restart chat')).not.toBeNull();
        expect(card(en.changes.resources.title).getByText(en.changes.live.byUnknown)).not.toBeNull();
    });

    it('should narrow the table to the changes picked and to what the search finds', async () => {
        serve(SERVED);
        const { getByRole, getByLabelText } = await panel('/changes', VIEWER);

        const updated = getByRole('button', { name: `${en.changes.resources.kinds.updated} 2` });
        fireEvent.click(updated);
        expect(updated.getAttribute('aria-pressed')).toBe('true');
        expect(rows()).toEqual(['qb-core', 'ox_lib']);

        fireEvent.click(getByRole('button', { name: `${en.changes.resources.kinds.didNotStart} 1` }));
        expect(rows()).toEqual(['qb-core', 'ox_lib', 'chat']);

        fireEvent.change(getByLabelText(en.changes.resources.search), { target: { value: '[ox]' } });
        expect(rows()).toEqual(['ox_lib']);

        fireEvent.change(getByLabelText(en.changes.resources.search), {
            target: { value: 'nothing like it' },
        });
        expect(rows()).toEqual([]);
        expect(document.body.textContent).toContain(en.changes.resources.noMatch);
    });

    it('should compare a start of the timeline with the one before it', async () => {
        const asked = serve(SERVED);
        const { getByRole, router } = await panel('/changes', VIEWER);

        fireEvent.click(getByRole('button', { name: /Scheduled restart/ }));
        await settle();

        expect(router.state.location.search).toEqual({ to: 2 });
        expect(asked).toContain('GET /servers/default/changes?to=2');
    });

    it('should read older starts only when asked, from where the newest page stops', async () => {
        const asked = serve({
            ...SERVED,
            'GET /servers/default/changes/starts': (query: URLSearchParams) =>
                query.get('before') === '2'
                    ? { starts: [BASELINE], next: null, check: null }
                    : { starts: [LATEST, RESTART], next: 2, check: null },
        });
        const { getByRole, queryByRole } = await panel('/changes', VIEWER);
        expect(starts()).toHaveLength(2);
        expect(asked.filter((request) => request.includes('/starts'))).toEqual([
            'GET /servers/default/changes/starts',
        ]);

        fireEvent.click(getByRole('button', { name: en.changes.starts.loadMore }));

        await waitFor(() => expect(starts()).toHaveLength(3));
        expect(asked).toContain('GET /servers/default/changes/starts?before=2');
        expect(queryByRole('button', { name: en.changes.starts.loadMore })).toBeNull();
    });

    it('should say snapshots start with the next start when no start was recorded yet', async () => {
        serve({
            ...SERVED,
            'GET /servers/default/changes': answer(404, { code: 'start_not_found' }),
            'GET /servers/default/changes/starts': { starts: [], next: null, check: null },
        });
        const { getByRole, getByText } = await panel('/changes', VIEWER);

        expect(getByRole('heading', { name: en.changes.empty.title })).not.toBeNull();
        expect(getByText(en.changes.empty.body)).not.toBeNull();
    });

    it('should read a first start as the baseline, never as every resource added', async () => {
        serve({
            ...SERVED,
            'GET /servers/default/changes': answer(404, { code: 'start_not_found' }),
            'GET /servers/default/changes/starts': { starts: [STARTS.starts[2]], next: null, check: null },
        });
        const { getByRole } = await panel('/changes', VIEWER);

        expect(getByRole('heading', { name: en.changes.empty.baselineTitle })).not.toBeNull();
        const start = card(en.changes.starts.title);
        expect(start.getByText(en.changes.baseline)).not.toBeNull();
        expect(start.queryByText(/added/)).toBeNull();
    });

    it('should lead the owner alone to the settings, now on the Settings page', async () => {
        serve(SERVED);
        await panel('/changes', VIEWER);
        expect(settingsLink()).toBeUndefined();
        cleanup();

        serve(SERVED);
        await panel('/changes', OWNER);
        expect(settingsLink()?.getAttribute('href')).toBe('/settings/data#setting-changesEnabled');
    });

    it('should count resources left unknown as checking, and let go of a change once none is listed', async () => {
        const hashing: Comparison = {
            ...COMPARISON,
            counts: { ...COMPARISON.counts, checking: 2 },
            resources: [
                ...COMPARISON.resources,
                changeOf({ name: 'qb-phone', change: 'checking' }),
                changeOf({ name: 'qb-garages', change: 'unknown' }),
            ],
        };
        serve({ ...SERVED, 'GET /servers/default/changes': hashing });
        const { getByRole, queryByRole, client } = await panel('/changes', VIEWER);

        fireEvent.click(getByRole('button', { name: `${en.changes.resources.kinds.checking} 2` }));
        expect(rows()).toEqual(['qb-phone', 'qb-garages']);

        // Both settle into changes the filter did not pick.
        act(() =>
            client.setQueryData(['changes', 'comparison', null, null], {
                ...hashing,
                counts: { ...COMPARISON.counts, checking: 0, updated: 4 },
                resources: [
                    ...COMPARISON.resources,
                    changeOf({ name: 'qb-phone', change: 'updated' }),
                    changeOf({ name: 'qb-garages', change: 'updated' }),
                ],
            }),
        );
        await settle();

        expect(queryByRole('button', { name: /^Checking/ })).toBeNull();
        expect(rows()).toHaveLength(8);
        expect(document.body.textContent).not.toContain(en.changes.resources.noMatch);
    });

    it('should show every change again when another comparison is picked', async () => {
        serve({
            ...SERVED,
            'GET /servers/default/changes': (query: URLSearchParams) =>
                query.get('to') === '2' ? { ...COMPARISON, from: BASELINE, to: RESTART } : COMPARISON,
        });
        const { getByRole, router } = await panel('/changes', VIEWER);

        fireEvent.click(getByRole('button', { name: `${en.changes.resources.kinds.updated} 2` }));
        expect(rows()).toEqual(['qb-core', 'ox_lib']);
        await act(() => router.navigate({ to: '/changes', search: { to: 2 } }));

        await waitFor(() => expect(rows()).toHaveLength(6));
        expect(
            getByRole('button', { name: `${en.changes.resources.kinds.updated} 2` }).getAttribute(
                'aria-pressed',
            ),
        ).toBe('false');
    });

    it('should write out the characters that reorder or hide a name the server reports', async () => {
        serve({
            ...SERVED,
            'GET /servers/default/changes': {
                ...COMPARISON,
                resources: [changeOf({ name: `qb-${RLO}eroc`, change: 'added' })],
            },
        });
        await panel('/changes', VIEWER);

        expect(rows()).toEqual(['qb-⟨U+202E⟩eroc']);
        expect(document.body.textContent).not.toContain(RLO);
    });

    it('should link a resource compared with the disk to the disk, not to a check the next one deletes', async () => {
        serve({ ...SERVED, 'GET /servers/default/changes': { ...COMPARISON, to: CHECK } });
        const { getByRole } = await panel('/changes?to=disk', VIEWER);

        const link = getByRole('link', { name: 'qb-inventory' });
        const search = new URL(link.getAttribute('href') ?? '', 'http://localhost').searchParams;
        expect(search.get('to')).toBe('disk');
        expect(search.get('from')).toBe('2');
    });

    it('should read the comparison with the disk again once a check of the disk is asked for', async () => {
        const asked = serve({
            ...SERVED,
            'GET /servers/default/changes': { ...COMPARISON, to: CHECK },
            'POST /servers/default/changes/check': { id: 10 },
        });
        const { getByRole } = await panel('/changes?to=disk', VIEWER);
        const compared = () => asked.filter((request) => request === 'GET /servers/default/changes?to=disk');
        expect(compared()).toHaveLength(1);

        fireEvent.click(getByRole('button', { name: en.changes.bar.check }));

        await waitFor(() => expect(compared()).toHaveLength(2));
        expect(asked).toContain('POST /servers/default/changes/check');
    });

    it('should read again what the latest start moved, once for a run of events', async () => {
        const asked = serve(SERVED);
        const { say } = await panel('/changes', VIEWER);
        const hashing = { ...LATEST, state: 'hashing' as const };
        say('snapshot', { latest: hashing, check: null });
        const before = asked.length;
        vi.useFakeTimers();

        for (const checking of [3, 2, 1]) {
            say('event', {
                latest: { ...hashing, counts: hashing.counts && { ...hashing.counts, checking } },
                check: null,
            });
            act(() => vi.advanceTimersByTime(500));
        }
        expect(asked).toHaveLength(before);
        await act(() => vi.advanceTimersByTimeAsync(REFRESH_MS));

        await act(() => vi.waitFor(() => expect(asked.length).toBeGreaterThanOrEqual(before + 2)));
        expect(asked.slice(before).toSorted()).toEqual([
            'GET /servers/default/changes',
            'GET /servers/default/changes/starts',
        ]);
    });
});

describe('ChangesResourcePage', () => {
    const config = `${INVENTORY.path}/config.lua`;
    const resource = `/changes/resource?${new URLSearchParams({ from: '2', to: '3', path: INVENTORY.path })}`;

    beforeAll(() => {
        HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
            this.open = true;
        };
        HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
            this.open = false;
        };
    });

    beforeEach(() => {
        vi.stubGlobal('scrollTo', () => {});
        // jsdom lays nothing out: the lines are drawn as if their box were a screen tall.
        vi.spyOn(HTMLElement.prototype, 'clientHeight', 'get').mockReturnValue(720);
    });

    afterEach(() => {
        cleanup();
        for (const live of links.splice(0)) {
            live.stop();
        }
        vi.useRealTimers();
        vi.unstubAllGlobals();
        vi.restoreAllMocks();
    });

    it('should give an account without the contents permission everything about a file but its lines', async () => {
        const written = { ...CLIENT, writtenBy: 'bob', writtenAt: '2026-10-31T13:55:00Z' };
        const asked = serve({
            ...SERVED,
            'GET /servers/default/changes/resource': {
                ...DETAIL,
                files: DETAIL.files.map((file) => (file.path === CLIENT.path ? written : file)),
            },
        });
        const { getByRole, getByText, queryByRole, findByText } = await panel(resource, VIEWER);

        const files = card(en.changes.resource.files);
        expect(files.getByText('main.lua')).not.toBeNull();
        expect(files.getByText(/\+3 −1/)).not.toBeNull();
        expect(files.queryByText('fxmanifest.lua')).toBeNull();
        expect(files.getByText(new RegExp(en.changes.diff.flagNames.afterStart))).not.toBeNull();
        expect(getByText(en.changes.resource.pickFileNoContent)).not.toBeNull();

        fireEvent.click(getByRole('button', { name: /^bag\.ydr/ }));
        expect(
            await findByText(`${en.changes.diff.binary} · 1.2 MB → 1.2 MB · ${en.changes.diff.sameSize}`),
        ).not.toBeNull();
        expect(
            getByRole('button', { name: `${en.common.copy}: ${en.changes.diff.shaAfter}` }),
        ).not.toBeNull();

        fireEvent.click(getByRole('button', { name: /^main\.lua/ }));
        expect(getByText(en.changes.resource.noContent)).not.toBeNull();
        expect(getByText(/^written over SFTP by bob at /)).not.toBeNull();
        expect(queryByRole('button', { name: en.changes.diff.viewFile })).toBeNull();
        expect(queryByRole('region', { name: /^Lines of / })).toBeNull();
        expect(asked.some((request) => /\/changes\/(diff|file)/.test(request))).toBe(false);
    });

    it('should open no file before one is picked, since reading one is audited', async () => {
        const asked = serve(SERVED);
        const { getByText } = await panel(resource, READER);

        expect(getByText(en.changes.resource.pickFile)).not.toBeNull();
        expect(asked.some((request) => request.includes('/changes/diff'))).toBe(false);
    });

    it('should show a hidden secret as a chip of its own words, and say when it changed', async () => {
        // Whatever the server puts in a secret's place, the chip never prints it.
        const leaked: FileDiff = {
            ...CONFIG_DIFF,
            diff: CONFIG_DIFF.diff && {
                ...CONFIG_DIFF.diff,
                hunks: CONFIG_DIFF.diff.hunks.map((hunk) => ({
                    ...hunk,
                    lines: hunk.lines.map((line) => ({
                        ...line,
                        segments: line.segments.map((segment) =>
                            segment.redacted
                                ? { ...segment, text: 'https://discord.com/api/webhooks/1/x' }
                                : segment,
                        ),
                    })),
                })),
            },
        };
        const asked = serve({ ...SERVED, 'GET /servers/default/changes/diff': leaked });
        const { getByRole, findByRole } = await panel(resource, READER);

        fireEvent.click(getByRole('button', { name: /^config\.lua/ }));
        const lines = await findByRole('region', { name: `Lines of ${config}` });

        const read = new URLSearchParams({
            from: '2',
            to: '3',
            path: config,
            page: '0',
            context: '3',
            whitespace: 'show',
        });
        expect(asked).toContain(`GET /servers/default/changes/diff?${read}`);
        const secrets = within(lines).getAllByText(en.changes.diff.hiddenSecret);
        expect(secrets).toHaveLength(2);
        expect(secrets[0]?.parentElement?.textContent).toBe(
            `${en.changes.diff.hiddenSecret}${en.changes.diff.redaction}`,
        );
        // A copy of the lines takes the code, not what a screen reader says of it.
        expect(secrets[0]?.className).toContain('select-none');
        expect(lines.textContent).not.toContain('discord.com');
        expect(within(lines).getAllByText(en.changes.diff.secretChanged)).toHaveLength(2);
        expect(within(lines).getByText(en.changes.diff.removed)).not.toBeNull();
        expect(within(lines).getByText(en.changes.diff.added)).not.toBeNull();
        expect(within(lines).getByText('@@ -11,3 +11,3 @@')).not.toBeNull();
        expect(document.body.textContent).toContain(en.changes.diff.flags.afterStart);
    });

    it('should show both hashes of a binary file that kept its size', async () => {
        serve(SERVED);
        const { getByRole, findByText } = await panel(resource, READER);

        fireEvent.click(getByRole('button', { name: /^bag\.ydr/ }));

        expect(
            await findByText(`${en.changes.diff.binary} · 1.2 MB → 1.2 MB · ${en.changes.diff.sameSize}`),
        ).not.toBeNull();
        expect(
            getByRole('button', { name: `${en.common.copy}: ${en.changes.diff.shaBefore}` }),
        ).not.toBeNull();
        expect(
            getByRole('button', { name: `${en.common.copy}: ${en.changes.diff.shaAfter}` }),
        ).not.toBeNull();
    });

    it('should purge the kept contents of a file once the owner confirms', async () => {
        const asked = serve({
            ...SERVED,
            'DELETE /servers/default/changes/contents': { versions: 3, bytes: 12_288 },
        });
        const { getByRole, findByText } = await panel(resource, OWNER);

        fireEvent.click(getByRole('button', { name: en.changes.resource.purge('config.lua') }));
        expect(getByRole('alertdialog').textContent).toContain(en.changes.resource.purgeBody);
        fireEvent.click(getByRole('button', { name: en.changes.resource.purgeConfirm }));

        expect(await findByText(en.changes.resource.purged('config.lua', 3, '12 KB'))).not.toBeNull();
        expect(asked).toContain(
            `DELETE /servers/default/changes/contents?${new URLSearchParams({ path: config })}`,
        );
    });

    it('should diff a file whose resource moved against where it was, and read it there for Before', async () => {
        const was = 'resources/[old]/qb-inventory/client/main.lua';
        const moved = { ...CLIENT, fromPath: was };
        const text: FileText = {
            path: was,
            snapshot: 2,
            text: {
                first: 1,
                lines: [[{ text: 'return {}', changed: false, redacted: false }]],
                total: 1,
                page: 0,
                pages: 1,
            },
        };
        const asked = serve({
            ...SERVED,
            'GET /servers/default/changes/resource': { ...DETAIL, files: [moved] },
            'GET /servers/default/changes/diff': diffOf(moved, [{ kind: 'added', text: 'return {}' }]),
            'GET /servers/default/changes/file': text,
        });
        const { getByRole, findByRole } = await panel(resource, READER);

        fireEvent.click(getByRole('button', { name: /^main\.lua/ }));
        await findByRole('region', { name: `Lines of ${CLIENT.path}` });
        expect(asked).toContain(
            `GET /servers/default/changes/diff?${new URLSearchParams({
                from: '2',
                to: '3',
                path: CLIENT.path,
                fromPath: was,
                page: '0',
                context: '3',
                whitespace: 'show',
            })}`,
        );

        fireEvent.click(getByRole('button', { name: en.changes.diff.viewFile }));
        fireEvent.click(getByRole('button', { name: en.changes.diff.before }));
        await waitFor(() =>
            expect(asked).toContain(
                `GET /servers/default/changes/file?${new URLSearchParams({ start: '2', path: was, page: '0' })}`,
            ),
        );
    });

    it('should say a renamed file kept its contents, without asking for a diff', async () => {
        const renamed = {
            ...CONFIG,
            path: `${INVENTORY.path}/settings.lua`,
            fromPath: config,
            change: 'renamed' as const,
            lines: null,
            flags: [],
        };
        const asked = serve({
            ...SERVED,
            'GET /servers/default/changes/resource': { ...DETAIL, files: [renamed] },
        });
        const { getByRole, getByText } = await panel(resource, READER);

        fireEvent.click(getByRole('button', { name: /^settings\.lua/ }));
        await settle();

        expect(getByText(en.changes.diff.renamed(config))).not.toBeNull();
        expect(asked.some((request) => request.includes('/changes/diff'))).toBe(false);
    });

    it('should show the characters that reorder code as escapes, warn about them, and keep each line apart', async () => {
        serve({
            ...SERVED,
            'GET /servers/default/changes/diff': diffOf(CONFIG, [
                { kind: 'context', text: 'local function isAdmin(role)' },
                { kind: 'added', text: `    if role ${RLO}~= "admin" then return true end` },
            ]),
        });
        const { getByRole, findByRole, getByText } = await panel(resource, READER);

        fireEvent.click(getByRole('button', { name: /^config\.lua/ }));
        const lines = await findByRole('region', { name: `Lines of ${config}` });

        expect(lines.textContent).not.toContain(RLO);
        expect(within(lines).getByText('⟨U+202E⟩')).not.toBeNull();
        expect(within(lines).getByText(en.changes.diff.invisible.bidi('U+202E'))).not.toBeNull();
        expect(getByText(en.changes.diff.invisibleWarning)).not.toBeNull();
        expect(
            within(lines).getByText('local function isAdmin(role)').closest('[class*="unicode-bidi"]'),
        ).not.toBeNull();
    });

    it('should say a file of a start still hashing is being read, then read it again once hashed', async () => {
        const hashing = { ...CLIENT, kept: false };
        let detail = { ...DETAIL, files: [hashing] };
        const asked = serve({ ...SERVED, 'GET /servers/default/changes/resource': () => detail });
        const { getByRole, getByText, say } = await panel(resource, READER);
        say('snapshot', { latest: { ...LATEST, state: 'hashing' }, check: null });

        fireEvent.click(getByRole('button', { name: /^main\.lua/ }));
        expect(getByText(en.changes.diff.beingRead)).not.toBeNull();
        expect(asked.some((request) => request.includes('/changes/diff'))).toBe(false);

        detail = { ...DETAIL, files: [CLIENT] };
        vi.useFakeTimers();
        say('event', { latest: LATEST, check: null });
        await act(() => vi.advanceTimersByTimeAsync(REFRESH_MS));

        // Each look moves the faked clock on, so the query cache passes on what it read.
        await act(() =>
            vi.waitFor(() => expect(asked.some((request) => request.includes('/changes/diff'))).toBe(true)),
        );
        expect(
            asked.filter((request) => request.startsWith('GET /servers/default/changes/resource')),
        ).toHaveLength(2);
    });
});
