import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
    Outlet,
    RouterProvider,
} from '@tanstack/react-router';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Me } from '../api/generated/Me';
import { MeContext } from '../auth/me';
import { ReauthDialog } from '../auth/ReauthDialog';
import { COMPARISON, CONFIG_DIFF, DETAIL, INVENTORY, SETTINGS, STARTS } from '../changes/fixtures';
import { changesSearch, resourceSearch } from '../changes/search';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime } from '../live/realtime';
import { ChangesPage } from './ChangesPage';
import { ChangesResourcePage } from './ChangesResourcePage';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

// Bit 0 the dashboard, 16 the changes, 17 their contents.
const OWNER: Me = { id: 1, username: 'ada', isOwner: true, permissions: '3ffff', mustChangePassword: false };
const VIEWER: Me = {
    id: 2,
    username: 'kenji',
    isOwner: false,
    permissions: '10001',
    mustChangePassword: false,
};
const READER: Me = {
    id: 3,
    username: 'bob',
    isOwner: false,
    permissions: '30001',
    mustChangePassword: false,
};

const answer = (status: number, body?: unknown) =>
    new Response(body === undefined ? null : JSON.stringify(body), {
        status,
        headers: { 'content-type': 'application/json' },
    });

/** Observer's answers by method and route; every request made is kept, with its query. */
const serve = (replies: Record<string, unknown>) => {
    const asked: string[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string, init: RequestInit) => {
            const url = new URL(input, 'http://localhost');
            const route = `${init.method ?? 'GET'} ${url.pathname.replace(/^\/api/, '')}`;
            asked.push(`${route}${url.search}`);
            const reply = replies[route];
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

/** The Changes pages at `path` for `me`, the socket never answering. */
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
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: () => ({ send: () => {}, close: () => {} }),
        refetch: () => {},
    });
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
    await settle();
    await settle();
    return { ...view, router };
};

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

/** The names in the resources table, in order. */
const rows = () =>
    [...document.querySelectorAll('tbody tr td:first-child a')].map((link) => link.textContent);

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

    it('should give the owner alone the settings, with the patterns that match no file', async () => {
        serve(SERVED);
        await panel('/changes', VIEWER);
        expect(document.body.textContent).not.toContain(en.changes.settings.title);
        cleanup();

        serve(SERVED);
        await panel('/changes', OWNER);
        const settings = card(en.changes.settings.title);
        expect(settings.getByText('*.secret', { selector: 'code' }).nextSibling?.textContent).toBe(
            en.changes.settings.matchesNone,
        );
        expect(settings.getByText('cache', { selector: 'code' }).nextSibling?.textContent).toBe(
            en.changes.settings.matchesNone,
        );
        expect(settings.getByText(en.changes.settings.matches(1))).not.toBeNull();
        expect((settings.getByLabelText(en.changes.settings.budget) as HTMLInputElement).value).toBe('2');
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
        vi.unstubAllGlobals();
        vi.restoreAllMocks();
    });

    it('should give an account without the contents permission the files and their counts, never a diff', async () => {
        const asked = serve(SERVED);
        const { getByText, queryByRole } = await panel(resource, VIEWER);

        const files = card(en.changes.resource.files);
        expect(files.getByText('main.lua')).not.toBeNull();
        expect(files.getByText(/\+3 −1/)).not.toBeNull();
        expect(files.queryByText('fxmanifest.lua')).toBeNull();
        expect(getByText(en.changes.resource.noContent)).not.toBeNull();
        expect(queryByRole('region', { name: /^Lines of / })).toBeNull();
        expect(asked.some((request) => request.includes('/changes/diff'))).toBe(false);
    });

    it('should show a hidden secret as a chip, and say when it changed', async () => {
        const asked = serve(SERVED);
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
        expect(secrets[0]?.parentElement?.textContent).toBe(`${en.changes.diff.hiddenSecret}[REDACTED]`);
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
});
