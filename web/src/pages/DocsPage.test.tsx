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
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { ApiError } from '../api/client';
import { integrationDocsQuery } from '../api/docs';
import type { IntegrationDocs } from '../api/generated/IntegrationDocs';
import type { Me } from '../api/generated/Me';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { MeContext } from '../auth/me';
import { integrationDocs } from '../docs/fixtures';
import { buildDocs, fieldsOf, handlerOf } from '../docs/model';
import { NOTED } from '../docs/Notes';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { PaletteContext } from '../shell/palette';
import { DocsPage } from './DocsPage';

/** The status the shell would follow, set by each test. */
const server = vi.hoisted(() => {
    const listeners = new Set<() => void>();
    const store = {
        status: null as ServerStatus | null,
        set: (next: ServerStatus | null) => {
            store.status = next;
            for (const listener of listeners) {
                listener();
            }
        },
        subscribe: (listener: () => void) => {
            listeners.add(listener);
            return () => listeners.delete(listener);
        },
    };
    return store;
});
vi.mock('../server/context', async () => {
    const { useSyncExternalStore } = await import('react');
    return {
        useServerStatus: () => ({
            status: useSyncExternalStore(server.subscribe, () => server.status),
            refused: false,
        }),
    };
});

/** A status with what the page reads of it. */
const status = (generation: number, state: ServerStatus['state'], startedAt: string | null = null) =>
    ({ generation, state, startedAt }) as ServerStatus;

const OWNER: Me = {
    id: 1,
    username: 'ada',
    isOwner: true,
    permissions: 'fffffff',
    mustChangePassword: false,
};
/** Sees the dashboard and nothing else: is told what the server does, and opens none of the pages named. */
const VIEWER: Me = { id: 2, username: 'grace', isOwner: false, permissions: '1', mustChangePassword: false };
/** Holds no permission: reads the contract, and is not told what the server does. */
const READER: Me = { id: 3, username: 'alan', isOwner: false, permissions: '0', mustChangePassword: false };

const words = en.docs;
const BRIDGE = 'sys_k3jx7qmw2pza';
const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));
const openPalette = vi.fn();
const writeText = vi.fn(async (_text: string) => {});

const open = async (
    docs: IntegrationDocs | ApiError,
    { me = OWNER, path = '/docs' }: { me?: Me; path?: string } = {},
) => {
    const client = new QueryClient({
        defaultOptions: { queries: { staleTime: Infinity, retry: false, retryOnMount: false } },
    });
    if (docs instanceof ApiError) {
        await client.prefetchQuery({ ...integrationDocsQuery, queryFn: () => Promise.reject(docs) });
    } else {
        client.setQueryData(integrationDocsQuery.queryKey, docs);
    }
    const root = createRootRoute({ component: Outlet });
    const router = createRouter({
        routeTree: root.addChildren([
            createRoute({ getParentRoute: () => root, path: '/docs', component: DocsPage }),
        ]),
        history: createMemoryHistory({ initialEntries: [path] }),
    });
    const view = render(
        <I18nProvider>
            <QueryClientProvider client={client}>
                <MeContext value={me}>
                    <PaletteContext value={openPalette}>
                        <RouterProvider router={router} />
                    </PaletteContext>
                </MeContext>
            </QueryClientProvider>
        </I18nProvider>,
    );
    await settle();
    await settle();
    return { ...view, client, router };
};

/** The card a section or an event has on the page. */
const card = (id: string) => document.getElementById(id) as HTMLElement;
const facts = () => within(card('this-server'));

describe('DocsPage', () => {
    beforeEach(() => {
        // jsdom lays nothing out, so it has no scrolling to the section a link leads to.
        Element.prototype.scrollIntoView = vi.fn();
        vi.stubGlobal('navigator', { ...navigator, clipboard: { writeText } });
        vi.stubGlobal(
            'fetch',
            vi.fn(async () => Response.json(integrationDocs())),
        );
        server.set(null);
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        vi.clearAllMocks();
    });

    it('should draw every section of the contract in its order, under contents that list them', async () => {
        const docs = integrationDocs();
        const model = buildDocs(docs);
        const { getAllByRole, getByRole } = await open(docs);

        expect(getByRole('heading', { level: 1 }).textContent).toBe(`${words.title} ${words.gloss}`);
        expect(getAllByRole('heading', { level: 2 }).map((heading) => heading.textContent)).toEqual([
            words.server.title,
            ...model.sections.map((section) => section.title),
        ]);
        const contents = within(getByRole('navigation', { name: words.toc }));
        const listed = contents
            .getAllByRole('link')
            .map((link) => [link.textContent, link.getAttribute('href')]);
        expect(listed[0]).toEqual([words.server.title, '#this-server']);
        for (const section of model.sections) {
            expect(listed).toContainEqual([section.title, `#${section.id}`]);
            for (const { name } of section.exports) {
                expect(listed).toContainEqual([name, `#${section.id}`]);
            }
        }
        expect(listed).toContainEqual(['shuttingDown', '#observershuttingdown']);
        // Each export shows where it is documented, with the parameters its script declares, on the
        // line that heads the section: the contract's own text may name it too.
        for (const section of model.sections.filter((documented) => documented.exports.length > 0)) {
            const heading = within(within(card(section.id)).getByText(words.exports));
            for (const { name, parameters } of section.exports) {
                expect(heading.getByText(`${name}(${parameters.join(', ')})`)).toBeTruthy();
            }
        }
        expect(listed).toHaveLength(
            1 +
                model.sections.length +
                model.sections.flatMap((section) => [...section.entries, ...section.exports]).length,
        );
    });

    it('should give each event the bridge raises its card: when it comes, its fields and a handler to copy', async () => {
        const docs = integrationDocs();
        const model = buildDocs(docs);
        const script = docs.example.map((file) => file.text).join('\n');
        await open(docs);

        expect(docs.events).not.toHaveLength(0);
        for (const event of docs.events) {
            const entry = model.sections
                .flatMap((section) => section.entries)
                .find((found) => found.title === event);
            const own = within(card(entry?.id ?? ''));

            expect(own.getByRole('heading', { level: 3 }).textContent, event).toBe(event);
            expect(own.getByText(words.event), event).toBeTruthy();
            for (const field of entry === undefined ? [] : fieldsOf(entry)) {
                expect(own.getAllByText(field).length, `${event} ${field}`).toBeGreaterThan(0);
            }
            const handler = entry === undefined ? '' : handlerOf(entry, script);
            expect(card(entry?.id ?? '').querySelector('pre')?.textContent, event).toBe(handler);
            fireEvent.click(own.getByRole('button', { name: words.copyNamed(words.handlerOf(event)) }));
            expect(writeText, event).toHaveBeenLastCalledWith(handler);
        }
        await settle();
    });

    it('should say what a server that does not run will do, and what one that runs does', async () => {
        const stopped = await open(integrationDocs(), { me: VIEWER });
        expect(facts().getByText(words.server.version(integrationDocs().contractVersion))).toBeTruthy();
        expect(facts().getByText(words.server.stopped)).toBeTruthy();
        expect(facts().getByText(words.server.bridgeStopped)).toBeTruthy();
        expect(facts().getByText(words.server.notSet)).toBeTruthy();
        expect(stopped.queryByRole('link', { name: words.server.launchSettings })).toBeNull();
        cleanup();

        await open(integrationDocs({ txadminEventsNext: true }));
        expect(facts().getByText(words.server.on)).toBeTruthy();
        expect(facts().getByText(words.server.txadminFromNext, { exact: false })).toBeTruthy();
        expect(facts().getByRole('link', { name: words.server.launchSettings }).getAttribute('href')).toBe(
            '/settings/launch#setting-txadminEvents',
        );
        cleanup();

        await open(
            integrationDocs({ bridge: BRIDGE, starting: true, txadminEvents: true, txadminEventsNext: true }),
        );
        expect(facts().getByText(words.server.running)).toBeTruthy();
        expect(facts().getByText(words.server.bridgeStarting)).toBeTruthy();
        expect(facts().getByText(BRIDGE)).toBeTruthy();
        cleanup();

        server.set(status(3, 'online', '2026-11-11T13:02:00Z'));
        await open(integrationDocs({ bridge: BRIDGE, txadminEvents: false, txadminEventsNext: true }));
        expect(facts().getByText(words.server.bridgeRunning)).toBeTruthy();
        expect(facts().getByText(words.server.startedAt('').trim(), { exact: false })).toBeTruthy();
        expect(facts().getByText(words.server.off)).toBeTruthy();
        // The setting was saved since this start.
        expect(
            facts().getByText(`${words.server.txadminOff} ${words.server.txadminNext(true)}`, {
                exact: false,
            }),
        ).toBeTruthy();
    });

    it('should add a word of the panel under the sections it has one for, each held to the contract', async () => {
        const docs = integrationDocs({ bridge: BRIDGE, txadminEvents: false, txadminEventsNext: false });
        const model = buildDocs(docs);
        const ids = model.sections.flatMap((section) => [
            section.id,
            ...section.entries.map((entry) => entry.id),
        ]);
        for (const noted of NOTED) {
            expect(ids, `CONTRACT.md has no heading at #${noted} any more`).toContain(noted);
        }

        await open(docs);
        const labels = within(card('player-labels'));
        expect(labels.getByText(words.notes.panel)).toBeTruthy();
        expect(labels.getByRole('link', { name: en.nav.players }).getAttribute('href')).toBe('/players');
        expect(
            within(card('observerstartchanges'))
                .getByRole('link', { name: en.nav.changes })
                .getAttribute('href'),
        ).toBe('/changes');
        const bans = within(card('bans'));
        expect(bans.getByRole('link', { name: en.nav.bans }).getAttribute('href')).toBe('/bans');
        expect(bans.getByRole('link', { name: words.notes.refusalMessage }).getAttribute('href')).toBe(
            '/settings/bans#setting-refusalMessage',
        );
        const txadmin = within(card('txadmins-events'));
        expect(txadmin.getByText(words.notes.server)).toBeTruthy();
        expect(txadmin.getByText(words.notes.txadminOff, { exact: false })).toBeTruthy();
        cleanup();

        // An account that opens none of those pages reads their names.
        await open(docs, { me: VIEWER });
        for (const noted of NOTED) {
            const footer = card(noted).querySelector('footer') as HTMLElement;
            expect(footer, noted).not.toBeNull();
            expect(within(footer).queryByRole('link'), noted).toBeNull();
        }
        expect(card('player-labels').querySelector('footer')?.textContent).toContain(en.nav.players);
    });

    it('should leave what the server does out for an account that is not told it', async () => {
        const docs = integrationDocs(null);
        const { getAllByRole, getByRole } = await open(docs, { me: READER });

        expect(card('this-server')).toBeNull();
        expect(card('txadmins-events').querySelector('footer')).toBeNull();
        expect(card('bans').querySelector('footer')).not.toBeNull();
        const contents = within(getByRole('navigation', { name: words.toc }));
        expect(contents.queryByRole('link', { name: words.server.title })).toBeNull();
        expect(getAllByRole('heading', { level: 2 }).map((heading) => heading.textContent)).toEqual(
            buildDocs(docs).sections.map((section) => section.title),
        );
    });

    it('should lead the links of the contract to their place on the page, and show the files it links to', async () => {
        const { getByRole } = await open(integrationDocs());

        const events = within(card('events'));
        expect(events.getAllByRole('link', { name: 'Bans' })[0]?.getAttribute('href')).toBe('#bans');
        expect(events.getByRole('link', { name: 'observer:playerRefused' }).getAttribute('href')).toBe(
            '#observerplayerrefused',
        );
        // Its own name, in its own card, is not a link to itself.
        expect(
            within(card('observerplayerrefused')).queryByRole('link', { name: 'observer:playerRefused' }),
        ).toBeNull();
        const example = within(card('example'));
        expect(example.getByRole('link', { name: 'observer_example' }).getAttribute('href')).toBe(
            '#files-observer_example',
        );
        const file = getByRole('button', { name: /^observer_example\/server\.lua/ });
        const code = document.getElementById(file.getAttribute('aria-controls') ?? '') as HTMLElement;
        expect(file.getAttribute('aria-expanded')).toBe('false');
        expect(code.hidden).toBe(true);
        fireEvent.click(file);
        expect(file.getAttribute('aria-expanded')).toBe('true');
        expect(code.hidden).toBe(false);
        expect(code.textContent).toContain("AddEventHandler('observer:shuttingDown'");
    });

    it('should open on the section its address names, and copy the address of a section', async () => {
        const { getByRole } = await open(integrationDocs(), { path: '/docs#bans' });

        expect(vi.mocked(Element.prototype.scrollIntoView).mock.contexts).toContain(card('bans'));
        fireEvent.click(getByRole('button', { name: words.copyLink('Bans') }));
        await settle();
        expect(writeText).toHaveBeenLastCalledWith(`${location.origin}/docs#bans`);
        expect(within(card('bans')).getByText(words.linkCopied)).toBeTruthy();
    });

    it('should give the link to read when the browser refuses to copy it', async () => {
        writeText.mockRejectedValueOnce(new Error('not allowed'));
        const { getByRole } = await open(integrationDocs());

        fireEvent.click(getByRole('button', { name: words.copyLink('Security') }));
        await settle();

        expect(
            within(card('security')).getByText(
                en.common.copyBlockedValue(`${location.origin}/docs#security`),
            ),
        ).toBeTruthy();
    });

    it('should open the search from its button, and say when the documentation cannot be read', async () => {
        const read = await open(integrationDocs());
        fireEvent.click(read.getByRole('button', { name: new RegExp(words.find) }));
        expect(openPalette).toHaveBeenCalledOnce();
        cleanup();

        const failed = await open(new ApiError(500, 'internal', {}));
        expect(failed.getByRole('alert').textContent).toContain(words.failed);
        expect(failed.queryByRole('navigation', { name: words.toc })).toBeNull();
    });

    it('should read what the server does again when it starts or stops, and not before', async () => {
        server.set(status(3, 'stopped'));
        const { client } = await open(integrationDocs());
        const invalidate = vi.spyOn(client, 'invalidateQueries');
        expect(invalidate).not.toHaveBeenCalled();

        act(() => server.set(status(4, 'starting')));
        await settle();

        expect(invalidate).toHaveBeenCalledExactlyOnceWith({ queryKey: integrationDocsQuery.queryKey });
    });
});
