import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Me } from '../api/generated/Me';
import type { ResourceRow } from '../api/generated/ResourceRow';
import type { ResourcesView } from '../api/generated/ResourcesView';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { ServerStatusProvider } from '../server/context';
import { ResourcesPage } from './ResourcesPage';

const words = en.resources;

const OWNER: Me = {
    id: 1,
    username: 'ada',
    isOwner: true,
    permissions: '7ffffff',
    mustChangePassword: false,
};
/** Sees the dashboard, and nothing else. */
const READER: Me = { id: 2, username: 'grace', isOwner: false, permissions: '1', mustChangePassword: false };

/** Half a minute past the minute, so that the time the page counts from never reads a minute short. */
const minutesAgo = (minutes: number) => new Date(Date.now() - minutes * 60_000 - 30_000).toISOString();

const row = (name: string, folder: string, changes: Partial<ResourceRow> = {}): ResourceRow => ({
    name,
    folder,
    origin: 'server',
    version: null,
    state: 'started',
    failedAt: null,
    warnings: null,
    dependants: [],
    controllable: true,
    last: null,
    asked: null,
    ...changes,
});

const rows = (): ResourceRow[] => [
    row('bad name', '[jobs]', { controllable: false }),
    row('chat', '', { origin: 'build', version: '1.0.0' }),
    row('docks', '[maps]', { warnings: { count: 3, heavy: 2, oversized: 1, missing: 1 } }),
    row('loadscreen', '', { version: '2.1.0' }),
    row('mechanic', '[jobs]', { state: 'stopped', failedAt: minutesAgo(31) }),
    row('ox_inventory', '[core]/[ox]', { version: '2.44.1' }),
    row('ox_lib', '[core]', { version: '3.30.6', dependants: ['ox_inventory', 'ox_target'] }),
    row('ox_target', '[core]/[ox]'),
    row('taxi', '[jobs]', {
        state: 'stopped',
        last: { at: minutesAgo(31), kind: 'stopped', by: 'maxime', with: null },
    }),
];

const listed = (resources: ResourceRow[] = rows(), changes: Partial<ResourcesView> = {}): ResourcesView => ({
    stage: 'listed',
    generation: 3,
    resources,
    truncated: false,
    rescan: null,
    rescanning: false,
    ...changes,
});

const STARTING: ServerStatus = {
    state: 'starting',
    cause: null,
    since: '2026-11-11T14:00:00Z',
    startedAt: null,
    generation: 3,
    problems: [],
    build: null,
    players: null,
    restartAt: null,
    launch: 'ready',
    name: null,
    port: null,
    start: {
        started: 212,
        failed: 0,
        found: 400,
        planned: 394,
        last: null,
        at: null,
        slowest: [],
        stuckAt: null,
        extensionMinutes: 15,
    },
    stopHolders: [],
    stop: null,
};

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** The orders the API was sent, each answered when the test says. */
const orders: { path: string; body: unknown; answer: (response: Response) => void }[] = [];

/** The page for `me`, with a socket the test speaks for. */
const page = async (me: Me = OWNER) => {
    let socket: SocketEvents | undefined;
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: () => {}, close: () => {} };
        },
        refetch: () => {},
    });
    const view = render(
        <QueryClientProvider client={new QueryClient()}>
            <I18nProvider>
                <MeContext value={me}>
                    <LiveContext value={live}>
                        <ServerStatusProvider>
                            <ResourcesPage />
                        </ServerStatusProvider>
                    </LiveContext>
                </MeContext>
            </I18nProvider>
        </QueryClientProvider>,
    );
    live.start();
    await settle();
    const tell = (message: unknown) => act(() => socket?.message(JSON.stringify(message)));
    tell({ type: 'hello', connectionId: 1, contract: 1 });
    const line = (name: string) => {
        const item = view.getByText(name, { selector: 'li span' }).closest('li');
        if (item === null) {
            throw new Error(`no row for ${name}`);
        }
        return within(item);
    };
    return {
        ...view,
        live,
        line,
        say: (type: 'snapshot' | 'event', data: ResourcesView) =>
            tell({ type, topic: 'server.resources', data }),
        status: (data: ServerStatus) => tell({ type: 'snapshot', topic: 'server.status', data }),
        press: async (button: HTMLElement) => {
            fireEvent.click(button);
            await settle();
        },
        answer: async (index: number, body: unknown, status = 200) => {
            orders[index]?.answer(Response.json(body, { status }));
            await settle();
        },
    };
};

const folders = (container: HTMLElement) =>
    [...container.querySelectorAll('section h2 button')].map((button) => button.textContent);

describe('ResourcesPage', () => {
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
        vi.stubGlobal(
            'fetch',
            (url: string, init?: RequestInit) =>
                new Promise<Response>((answer) => {
                    const body: unknown =
                        init?.body === undefined ? undefined : JSON.parse(String(init.body));
                    orders.push({ path: url, body, answer });
                }),
        );
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        orders.length = 0;
        localStorage.clear();
    });

    it('should list the resources by folder, the server root first and the build last, with what each filter counts', async () => {
        const { container, say, line, getByRole, live } = await page();
        say('snapshot', listed());

        expect(folders(container)).toEqual([
            `resources${words.folder.started(1, 1)}`,
            `[core]${words.folder.started(1, 1)}`,
            `[core]/[ox]${words.folder.started(2, 2)}`,
            `[jobs]${words.folder.started(1, 3)}`,
            `[maps]${words.folder.started(1, 1)}`,
            `system_resources${words.folder.build}${words.folder.started(1, 1)}`,
        ]);
        const filters = within(getByRole('group', { name: words.filtersLabel }));
        expect(filters.getAllByRole('button').map((button) => button.textContent)).toEqual([
            `${words.filters.all}9`,
            `${words.filters.started}7`,
            `${words.filters.stopped}1`,
            `${words.filters.failed}1`,
            `${words.filters.warned}1`,
        ]);
        expect(line('ox_lib').getByText('3.30.6')).not.toBeNull();
        expect(line('ox_lib').getByText(words.states.started)).not.toBeNull();
        expect(line('mechanic').getByText(words.didNotStart)).not.toBeNull();
        expect(line('mechanic').getByText('FXServer could not start it, 31 min ago')).not.toBeNull();
        expect(line('taxi').getByText('stopped by maxime, 31 min ago')).not.toBeNull();
        live.stop();
    });

    it('should narrow the list by filter and by name, and say when nothing is left', async () => {
        const { container, say, getByRole, getByText, queryByText, live } = await page();
        say('snapshot', listed());

        fireEvent.click(getByRole('button', { name: new RegExp(`^${words.filters.failed}`) }));
        expect(folders(container)).toEqual([`[jobs]${words.folder.started(1, 3)}`]);
        expect(queryByText('taxi')).toBeNull();

        fireEvent.click(getByRole('button', { name: new RegExp(`^${words.filters.all}`) }));
        fireEvent.change(getByRole('searchbox', { name: words.find }), { target: { value: ' OX_ ' } });
        expect(folders(container).map((folder) => folder?.split(/\d/)[0])).toEqual(['[core]', '[core]/[ox]']);

        fireEvent.change(getByRole('searchbox', { name: words.find }), { target: { value: 'nothing' } });
        expect(getByText(words.noMatch)).not.toBeNull();
        live.stop();
    });

    it('should fold a folder on a click, and find it folded when the page opens again', async () => {
        const first = await page();
        first.say('snapshot', listed());

        fireEvent.click(first.getByRole('button', { name: /^\[jobs\]/ }));
        expect(first.queryByText('taxi')).toBeNull();
        expect(first.getByRole('button', { name: /^\[jobs\]/ }).getAttribute('aria-expanded')).toBe('false');
        expect(first.getByText('ox_lib')).not.toBeNull();
        first.live.stop();
        cleanup();

        const again = await page();
        again.say('snapshot', listed());
        expect(again.queryByText('taxi')).toBeNull();
        fireEvent.click(again.getByRole('button', { name: /^\[jobs\]/ }));
        expect(again.getByText('taxi')).not.toBeNull();
        again.live.stop();
    });

    it('should start a resource at once, say who asked until the list has it, and take no other order meanwhile', async () => {
        const { say, line, press, answer, live } = await page();
        say('snapshot', listed());

        await press(line('taxi').getByRole('button', { name: words.verbs.start }));

        expect(orders.map((order) => [order.path, order.body])).toEqual([
            ['/api/servers/default/resources/start', { name: 'taxi', dependants: false }],
        ]);
        expect(line('taxi').getByText(words.asked.start('ada'))).not.toBeNull();
        const start = line('taxi').getByRole('button', { name: words.verbs.start });
        expect([start.hasAttribute('disabled'), start.getAttribute('aria-busy')]).toEqual([true, 'true']);

        await answer(0, { delivery: 'written', dependants: [] });
        const started = rows().map((each) =>
            each.name === 'taxi'
                ? {
                      ...each,
                      state: 'started' as const,
                      last: { ...each.last!, kind: 'started' as const, by: 'ada' },
                  }
                : each,
        );
        say('event', listed(started));
        expect(line('taxi').queryByRole('button', { name: words.verbs.start })).toBeNull();
        expect(line('taxi').getByRole('button', { name: words.verbs.stop }).hasAttribute('disabled')).toBe(
            false,
        );
        live.stop();
    });

    it('should name what stops with a resource before it stops it', async () => {
        const { say, line, press, getByRole, queryByRole, live } = await page();
        say('snapshot', listed());

        await press(line('ox_lib').getByRole('button', { name: words.verbs.stop }));
        expect(orders).toHaveLength(0);
        const dialog = within(getByRole('alertdialog'));
        expect(dialog.getAllByRole('listitem').map((item) => item.textContent)).toEqual([
            'ox_inventory',
            'ox_target',
        ]);
        expect(dialog.getByText(words.stayStopped(2))).not.toBeNull();

        await press(dialog.getByRole('button', { name: words.confirmStop('ox_lib', 3) }));

        expect(queryByRole('alertdialog')).toBeNull();
        expect(orders.map((order) => [order.path, order.body])).toEqual([
            ['/api/servers/default/resources/stop', { name: 'ox_lib', dependants: false }],
        ]);
        expect(line('ox_lib').getByText(words.asked.stop('ada'))).not.toBeNull();
        live.stop();
    });

    it('should offer to start again what a restart stops, and restart the resource alone when told to', async () => {
        const { say, line, press, getByRole, live } = await page();
        say('snapshot', listed());

        await press(line('ox_lib').getByRole('button', { name: words.verbs.restart }));
        const dialog = within(getByRole('alertdialog'));
        const again = dialog.getByRole('checkbox', { name: words.again(2, 'ox_lib') });
        expect(again).toHaveProperty('checked', true);
        expect(dialog.getByRole('button', { name: words.confirmRestart('ox_lib', 3) })).not.toBeNull();
        await press(dialog.getByRole('button', { name: en.common.cancel }));
        expect(orders).toHaveLength(0);

        await press(line('ox_lib').getByRole('button', { name: words.verbs.restart }));
        await press(
            within(getByRole('alertdialog')).getByRole('button', { name: words.confirmRestart('ox_lib', 3) }),
        );
        expect(orders[0]?.body).toEqual({ name: 'ox_lib', dependants: true });
        live.stop();
    });

    it('should restart a resource without the others when the box is cleared', async () => {
        const { say, line, press, getByRole, live } = await page();
        say('snapshot', listed());

        await press(line('ox_lib').getByRole('button', { name: words.verbs.restart }));
        const dialog = within(getByRole('alertdialog'));
        fireEvent.click(dialog.getByRole('checkbox', { name: words.again(2, 'ox_lib') }));
        await press(dialog.getByRole('button', { name: words.confirmRestart('ox_lib', 1) }));

        expect(orders.map((order) => [order.path, order.body])).toEqual([
            ['/api/servers/default/resources/restart', { name: 'ox_lib', dependants: false }],
        ]);
        live.stop();
    });

    it('should show an order anyone made until FXServer answers it, its buttons waiting', async () => {
        const { say, line, live } = await page();
        const asked = rows().map((each) =>
            each.name === 'ox_target'
                ? { ...each, asked: { kind: 'restarted' as const, by: 'maxime' } }
                : each,
        );
        say('snapshot', listed(asked));

        expect(line('ox_target').getByText(words.asked.restart('maxime'))).not.toBeNull();
        const restart = line('ox_target').getByRole('button', { name: words.verbs.restart });
        const stop = line('ox_target').getByRole('button', { name: words.verbs.stop });
        expect([restart.getAttribute('aria-busy'), stop.hasAttribute('disabled')]).toEqual(['true', true]);
        expect(
            line('ox_inventory').getByRole('button', { name: words.verbs.stop }).hasAttribute('disabled'),
        ).toBe(false);
        live.stop();
    });

    it('should leave who only reads the list without a button, and one it cannot name to the console', async () => {
        const reader = await page(READER);
        reader.say('snapshot', listed());
        expect(reader.queryByRole('button', { name: words.rescan })).toBeNull();
        expect(reader.queryByRole('button', { name: words.verbs.start })).toBeNull();
        expect(reader.queryByRole('button', { name: words.verbs.restart })).toBeNull();
        expect(reader.line('taxi').getByText(words.states.stopped)).not.toBeNull();
        reader.live.stop();
        cleanup();

        const owner = await page();
        owner.say('snapshot', listed());
        expect(owner.line('bad name').queryByRole('button', { name: words.verbs.stop })).toBeNull();
        expect(owner.line('bad name').getByRole('button', { name: words.consoleOnly })).not.toBeNull();
        owner.live.stop();
    });

    it('should tell what FXServer warned of as it started a resource', async () => {
        const { say, line, getByRole, live } = await page();
        say('snapshot', listed());

        fireEvent.click(line('docks').getByRole('button', { name: words.warnings(3) }));

        expect(getByRole('tooltip').textContent).toBe(
            `${words.warned(3)}${words.heavy(2, 1)}${words.missing(1)}`,
        );
        live.stop();
    });

    it('should have FXServer read its folders again, and say what it found', async () => {
        const { say, press, answer, getByRole, getByText, live } = await page();
        say('snapshot', listed());

        await press(getByRole('button', { name: words.rescan }));
        expect(orders.map((order) => [order.path, order.body])).toEqual([
            ['/api/servers/default/resources/rescan', undefined],
        ]);
        expect(getByRole('button', { name: words.rescan }).getAttribute('aria-busy')).toBe('true');

        await answer(0, { delivery: 'written', dependants: [] });
        const found = [...rows(), row('bank', '[jobs]', { state: 'stopped' })];
        say(
            'event',
            listed(found, {
                rescan: { at: minutesAgo(0), by: 'ada', added: ['bank'], addedCount: 1, changed: 2 },
            }),
        );

        expect(
            getByText(
                (_, element) =>
                    element?.textContent ===
                    `FXServer read its folders again: 1 new resource, bank, and 2 changed manifests.`,
                { selector: '[role="status"] div' },
            ),
        ).not.toBeNull();
        expect(getByRole('button', { name: words.rescan }).getAttribute('aria-busy')).toBeNull();
        live.stop();
    });

    it('should say why there is no list: no server, a server that starts, a bridge that lists nothing', async () => {
        const { say, status, getByRole, getByText, queryByRole, live } = await page();

        say('snapshot', listed([], { stage: 'stopped', generation: 0 }));
        expect(getByRole('heading', { name: words.empty.stopped.title })).not.toBeNull();
        expect(queryByRole('button', { name: words.rescan })).toBeNull();

        status(STARTING);
        say('event', listed([], { stage: 'starting' }));
        expect(getByRole('heading', { name: words.empty.starting.title })).not.toBeNull();
        expect(getByText(words.empty.starting.text(212, 394))).not.toBeNull();

        say('event', listed([], { stage: 'unreported' }));
        expect(getByText(words.empty.unreported.text)).not.toBeNull();

        say('event', listed([]));
        expect(getByText(words.none)).not.toBeNull();
        live.stop();
    });

    it('should say what an order ran into, and let go of it', async () => {
        const { say, line, press, answer, getByRole, queryByRole, live } = await page();
        say('snapshot', listed());

        await press(line('taxi').getByRole('button', { name: words.verbs.start }));
        await answer(0, { code: 'resource_changed' }, 409);

        const alert = getByRole('alert');
        expect(alert.textContent).toBe(
            words.orderFailed(words.ordering.start('taxi'), en.errors.resource_changed),
        );
        expect(line('taxi').getByRole('button', { name: words.verbs.start }).hasAttribute('disabled')).toBe(
            false,
        );

        fireEvent.click(within(alert).getByRole('button', { name: en.controls.dismiss }));
        expect(queryByRole('alert')).toBeNull();

        await press(line('taxi').getByRole('button', { name: words.verbs.start }));
        await answer(1, { delivery: 'busy', dependants: [] });
        expect(getByRole('alert').textContent).toBe(en.console.busy);
        live.stop();
    });
});
