import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { catalogQuery } from '../api/artifacts';
import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import type { CfgReport } from '../api/generated/CfgReport';
import type { FolderReport } from '../api/generated/FolderReport';
import type { ServerStatus } from '../api/generated/ServerStatus';
import type { StoredLaunch } from '../api/generated/StoredLaunch';
import { DEFAULT_LAUNCH, launchQuery } from '../api/launch';
import { ArtifactsProvider } from '../artifacts/context';
import { ARTIFACTS } from '../artifacts/fixtures';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { ScheduleProvider } from '../schedule/context';
import { ServerStatusProvider } from '../server/context';
import { CFG, DATA_PATH, FOLDER, finding } from '../setup/fixtures';
import { ServerSetupPage } from './ServerSetupPage';

// The page links to the Artifacts page and the console; no router runs here.
vi.mock('@tanstack/react-router', () => ({
    Link: ({ to, children, className }: { to: string; children: ReactNode; className?: string }) => (
        <a href={to} className={className}>
            {children}
        </a>
    ),
}));

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const words = en.serverSetup;

const STOPPED: ServerStatus = {
    state: 'stopped',
    cause: null,
    since: '2026-10-30T14:00:00Z',
    startedAt: null,
    generation: 0,
    problems: [],
    build: null,
    players: null,
    restartAt: null,
    launch: 'notConfigured',
};

const FIXTURE = structuredClone(ARTIFACTS) as unknown as ArtifactsStatus;
/** 37900 installed and selected, nothing under way. */
const BUILDS: ArtifactsStatus = { ...FIXTURE, pending: null, job: null, running: null, probation: null };

const stored = (version: number): StoredLaunch => ({
    settings: { ...DEFAULT_LAUNCH, dataPath: DATA_PATH },
    version,
    legacyBuildPath: null,
});

type Answer = (body: unknown) => Response;

/**
 * Answers the API from `routes`, by method and path; a list answers its calls in turn, its last
 * answer repeated. Says what each request sent.
 */
const serve = (routes: Record<string, Answer | Answer[]>) => {
    const sent: { route: string; body: unknown }[] = [];
    const turns = new Map<string, number>();
    vi.stubGlobal(
        'fetch',
        vi.fn(async (url: string, init: RequestInit = {}) => {
            const route = `${init.method ?? 'GET'} ${url.replace(/^\/api/, '')}`;
            const body: unknown = init.body === undefined ? undefined : JSON.parse(String(init.body));
            sent.push({ route, body });
            const answers = routes[route];
            if (answers === undefined) {
                return Response.json({ code: 'not_found' }, { status: 404 });
            }
            const list = Array.isArray(answers) ? answers : [answers];
            const turn = turns.get(route) ?? 0;
            turns.set(route, turn + 1);
            return (list[Math.min(turn, list.length - 1)] as Answer)(body);
        }),
    );
    return {
        sent,
        of: (route: string) => sent.filter((each) => each.route === route).map((each) => each.body),
    };
};

const json = (value: unknown) => () => Response.json(value);
const FOLDER_ROUTE = 'POST /servers/default/setup/folder';
const CFG_ROUTE = 'POST /servers/default/setup/cfg';
const SAVE_ROUTE = 'PUT /servers/default/setup';

/** The page over settings already read, for the owner, the builds and FXServer as the topics say. */
const page = async ({
    saved = null,
    builds = BUILDS,
    status = STOPPED,
}: { saved?: StoredLaunch | null; builds?: ArtifactsStatus; status?: ServerStatus } = {}) => {
    let socket: SocketEvents | undefined;
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: () => {}, close: () => {} };
        },
        refetch: () => {},
    });
    const client = new QueryClient({
        defaultOptions: { queries: { staleTime: Infinity, retry: false }, mutations: { retry: false } },
    });
    client.setQueryData(launchQuery.queryKey, saved);
    client.setQueryData(catalogQuery.queryKey, {
        platform: 'windows',
        fetchedAt: '2026-10-30T14:00:00Z',
        builds: [],
        warnings: [],
    });
    const view = render(
        <QueryClientProvider client={client}>
            <I18nProvider>
                <MeContext
                    value={{
                        id: 1,
                        username: 'ada',
                        isOwner: true,
                        permissions: 'fff',
                        mustChangePassword: false,
                    }}
                >
                    <LiveContext value={live}>
                        <ServerStatusProvider>
                            <ScheduleProvider>
                                <ArtifactsProvider>
                                    <ServerSetupPage />
                                </ArtifactsProvider>
                            </ScheduleProvider>
                        </ServerStatusProvider>
                    </LiveContext>
                </MeContext>
            </I18nProvider>
        </QueryClientProvider>,
    );
    live.start();
    await settle();
    const say = (message: unknown) => act(() => socket?.message(JSON.stringify(message)));
    say({ type: 'hello', connectionId: 1, contract: 1 });
    say({ type: 'snapshot', topic: 'server.status', data: status });
    say({ type: 'snapshot', topic: 'server.artifacts', data: builds });
    const button = (name: string) => view.getByRole('button', { name }) as HTMLButtonElement;
    const press = async (name: string) => {
        fireEvent.click(button(name));
        await settle();
    };
    /** Through the folder step, with the folder accepted. */
    const toCfg = async () => {
        fireEvent.change(view.getByRole('textbox', { name: words.folder.label }), {
            target: { value: DATA_PATH },
        });
        await press(words.folder.check);
        await press(words.continue);
    };
    return { ...view, client, say, button, press, toCfg };
};

describe('ServerSetupPage', () => {
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
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should check the folder, take the cfg found there, and check it on entering its step', async () => {
        const live: FolderReport = {
            ...FOLDER,
            cfgFiles: ['configs/live.cfg'],
            detectedCfg: 'configs/live.cfg',
        };
        const api = serve({ [FOLDER_ROUTE]: json(live), [CFG_ROUTE]: json(CFG) });
        const { button, getByText, getByRole, toCfg } = await page();

        expect(button(words.continue).disabled).toBe(true);
        getByText(words.folder.notChecked);
        await toCfg();

        expect(api.of(FOLDER_ROUTE)).toEqual([{ dataPath: DATA_PATH }]);
        expect(api.of(CFG_ROUTE)).toEqual([
            { dataPath: DATA_PATH, cfgFile: 'configs/live.cfg', onesync: 'on' },
        ]);
        expect(document.activeElement).toBe(getByRole('heading', { name: words.stepNames.cfg }));
        const steps = within(getByRole('list', { name: words.steps })).getAllByRole('listitem');
        expect(steps[1]?.getAttribute('aria-current')).toBe('step');
        expect(steps[0]?.textContent).toContain(words.said.passed);
    });

    it('should say why a folder is refused and offer the one nearby that fits', async () => {
        const resources = `${DATA_PATH}\\resources`;
        const refused: FolderReport = {
            ...FOLDER,
            problem: 'isResources',
            suggestion: DATA_PATH,
            resources: null,
            cfgFiles: [],
            detectedCfg: null,
        };
        const api = serve({ [FOLDER_ROUTE]: [json(refused), json(FOLDER)] });
        const { getByRole, button, press } = await page();
        const field = getByRole('textbox', { name: words.folder.label }) as HTMLInputElement;

        fireEvent.change(field, { target: { value: resources } });
        await press(words.folder.check);

        expect(field.getAttribute('aria-invalid')).toBe('true');
        expect(field.getAttribute('aria-describedby')).toContain('error');
        expect(document.getElementById(field.getAttribute('aria-describedby') ?? '')?.textContent).toBe(
            words.folder.problems.isResources,
        );
        expect(button(words.continue).disabled).toBe(true);

        await press(words.folder.useSuggestion);

        expect(field.value).toBe(DATA_PATH);
        expect(api.of(FOLDER_ROUTE)).toEqual([{ dataPath: resources }, { dataPath: DATA_PATH }]);
        expect(button(words.continue).disabled).toBe(false);
    });

    it('should keep Continue off while a finding blocks, each with its file and line', async () => {
        const blocked: CfgReport = {
            ...CFG,
            findings: [
                finding('endpointOnPanel', 'blocking', 2, { protocol: 'tcp', address: '0.0.0.0:40125' }),
                ...CFG.findings,
            ],
        };
        serve({ [FOLDER_ROUTE]: json(FOLDER), [CFG_ROUTE]: [json(blocked), json(CFG)] });
        const { button, getByText, getByRole, press, toCfg } = await page();

        await toCfg();

        expect(button(words.continue).disabled).toBe(true);
        getByText(words.cfg.blocked);
        const group = getByRole('heading', { name: words.cfg.groups.blocking(1) }).closest(
            'section',
        ) as HTMLElement;
        const row = within(group).getAllByRole('listitem')[0] as HTMLElement;
        expect(within(row).getByText(words.cfg.severities.blocking)).toBeTruthy();
        within(row).getByText(words.cfg.kinds.endpointOnPanel({ protocol: 'tcp', address: '0.0.0.0:40125' }));
        within(row).getByText('server.cfg:2');

        await press(words.cfg.checkAgain);

        expect(button(words.continue).disabled).toBe(false);
    });

    it("should follow the cfg's OneSync until the owner picks one, checking the cfg again with it", async () => {
        const legacy: CfgReport = {
            ...CFG,
            facts: { ...(CFG.facts as NonNullable<CfgReport['facts']>), onesync: 'legacy' },
        };
        const api = serve({ [FOLDER_ROUTE]: json(FOLDER), [CFG_ROUTE]: json(legacy) });
        const { getByRole, press, toCfg } = await page({
            saved: { ...stored(3), settings: { ...stored(3).settings, onesync: 'off' } },
        });

        await toCfg();
        await waitFor(() => expect(api.of(CFG_ROUTE)).toHaveLength(2));
        await press(words.continue);
        await press(words.continue);

        expect(api.of(CFG_ROUTE).map((body) => (body as { onesync: string }).onesync)).toEqual([
            'off',
            'legacy',
        ]);
        expect(
            (getByRole('radio', { name: en.server.onesyncModes.legacy }) as HTMLInputElement).checked,
        ).toBe(true);
    });

    it('should only save once the build step is skipped, and say why', async () => {
        const api = serve({
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: json(CFG),
            [SAVE_ROUTE]: json(stored(1)),
        });
        const { client, getByText, queryByRole, press, toCfg } = await page({
            builds: { ...BUILDS, selected: null },
        });

        await toCfg();
        await press(words.continue);
        await press(words.build.skip);

        getByText(words.start.withoutBuild);
        expect(queryByRole('button', { name: words.start.saveAndStart })).toBeNull();
        await press(words.start.saveOnly);

        await waitFor(() => getByText(en.server.saved));
        expect(api.of(SAVE_ROUTE)).toEqual([
            { settings: { ...DEFAULT_LAUNCH, dataPath: DATA_PATH, cfgFile: 'server.cfg' }, version: null },
        ]);
        expect(api.of('POST /servers/default/start')).toEqual([]);
        expect(client.getQueryData(launchQuery.queryKey)).toEqual(stored(1));
    });

    it('should save, start FXServer and follow its first launch until it is ready', async () => {
        const api = serve({
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: json(CFG),
            [SAVE_ROUTE]: json(stored(1)),
            'POST /servers/default/start': () => new Response(null, { status: 204 }),
        });
        const { getByRole, getByText, press, say, toCfg } = await page();

        await toCfg();
        await press(words.continue);
        await press(words.continue);
        await press(words.start.saveAndStart);
        await waitFor(() => getByText(words.start.starting));

        expect(api.sent.map((each) => each.route).slice(-2)).toEqual([
            SAVE_ROUTE,
            'POST /servers/default/start',
        ]);
        const launch = () => within(getByRole('list', { name: words.launch.title })).getAllByRole('listitem');
        expect(launch()[0]?.getAttribute('aria-current')).toBe('step');

        say({
            type: 'event',
            topic: 'server.status',
            data: { ...STOPPED, state: 'starting', generation: 1, cause: { kind: 'requested', by: 'ada' } },
        });
        expect(launch()[1]?.getAttribute('aria-current')).toBe('step');
        expect(launch()[0]?.textContent).toContain(words.said.passed);

        say({
            type: 'event',
            topic: 'server.status',
            data: { ...STOPPED, state: 'online', generation: 1, cause: { kind: 'ready' }, launch: 'ready' },
        });
        expect(launch().every((step) => step.textContent?.includes(words.said.passed))).toBe(true);
        getByText(words.launch.ready);
    });

    it('should name the step that failed, and keep it once FXServer is restarted after the crash', async () => {
        serve({
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: json(CFG),
            [SAVE_ROUTE]: json(stored(1)),
            'POST /servers/default/start': () => new Response(null, { status: 204 }),
        });
        const { getByRole, getByText, press, say, toCfg } = await page();
        await toCfg();
        await press(words.continue);
        await press(words.continue);
        await press(words.start.saveAndStart);
        await waitFor(() => getByText(words.start.starting));

        const run = { ...STOPPED, generation: 1, since: '2026-10-30T14:00:05Z' };
        say({
            type: 'event',
            topic: 'server.status',
            data: { ...run, state: 'crashed', cause: { kind: 'bindError', address: '0.0.0.0:30120' } },
        });
        say({
            type: 'event',
            topic: 'server.status',
            data: { ...run, state: 'starting', generation: 2, cause: { kind: 'recovery' } },
        });

        const port = within(getByRole('list', { name: words.launch.title })).getAllByRole('listitem')[1];
        expect(port?.textContent).toContain(words.said.failed);
        getByText(en.server_state.bindError('0.0.0.0:30120'));
    });

    it('should go back to the cfg, checked again, when the save finds it now blocks', async () => {
        const api = serve({
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: json(CFG),
            [SAVE_ROUTE]: () =>
                Response.json(
                    { code: 'setup_blocked', params: { findings: ['noEndpoint'] } },
                    { status: 422 },
                ),
        });
        const { getByRole, getByText, press, toCfg } = await page();
        await toCfg();
        await press(words.continue);
        await press(words.continue);

        await press(words.start.saveAndStart);

        await waitFor(() => getByText(en.errors.setup_blocked({ findings: ['noEndpoint'] })));
        expect(document.activeElement).toBe(getByRole('heading', { name: words.stepNames.cfg }));
        expect(api.of(CFG_ROUTE)).toHaveLength(2);
        expect(api.of('POST /servers/default/start')).toEqual([]);
    });

    it('should start again from the settings saved elsewhere when its version is stale', async () => {
        serve({
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: json(CFG),
            [SAVE_ROUTE]: () => Response.json({ code: 'conflict' }, { status: 409 }),
            'GET /servers/default/launch': json(stored(4)),
        });
        const { getByText, press, toCfg } = await page({ saved: stored(3) });
        await toCfg();
        await press(words.continue);
        await press(words.continue);

        await press(words.start.saveAndStart);

        await waitFor(() => getByText(words.changedElsewhere));
        await press(words.startAgain);
        getByText(words.folder.notChecked);
    });
});
