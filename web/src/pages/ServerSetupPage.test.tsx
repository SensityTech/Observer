import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import { catalogQuery } from '../api/artifacts';
import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import type { CfgReport } from '../api/generated/CfgReport';
import type { Discovery } from '../api/generated/Discovery';
import type { FolderListing } from '../api/generated/FolderListing';
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

// The page links to the Artifacts page, the console and the Server page; no router runs here.
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

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const words = en.serverSetup;

/** What a field says below it: its error, in place of its hint. */
const described = (field: HTMLElement) =>
    document.getElementById(field.getAttribute('aria-describedby') ?? '')?.textContent;

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
    name: null,
    port: null,
    start: null,
};

const FIXTURE = structuredClone(ARTIFACTS) as unknown as ArtifactsStatus;
/** 37900 installed and selected, nothing under way. */
const BUILDS: ArtifactsStatus = { ...FIXTURE, pending: null, job: null, running: null, probation: null };

const stored = (version: number): StoredLaunch => ({
    settings: { ...DEFAULT_LAUNCH, dataPath: DATA_PATH },
    version,
    legacyBuildPath: null,
});

type Answer = (body: unknown) => Response | Promise<Response>;

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
            const answers = routes[route] ?? (route === DISCOVER_ROUTE ? json(NOTHING_FOUND) : undefined);
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
const DISCOVER_ROUTE = 'POST /servers/default/setup/discover';
const FOLDERS_ROUTE = 'POST /servers/default/setup/folders';

const PLACES: Discovery['places'] = [
    { kind: 'drive', path: 'C:\\' },
    { kind: 'drive', path: 'D:\\' },
    { kind: 'home', path: 'C:\\Users\\Ada' },
];
/** What the search finds on a host without a server: the places to browse from alone. */
const NOTHING_FOUND: Discovery = { places: PLACES, servers: [], truncated: false };
const LIVE: Discovery['servers'][number] = {
    path: DATA_PATH,
    resources: 54,
    resourcesTruncated: false,
    cfg: 'server.cfg',
    onesync: 'on',
    txAdmin: true,
    port: 30120,
    portHolder: { program: 'FXServer.exe', fxServer: true },
};
const LAB: Discovery['servers'][number] = {
    path: 'D:\\FiveM\\lab-server',
    resources: 37,
    resourcesTruncated: false,
    cfg: 'server.cfg',
    onesync: 'off',
    txAdmin: false,
    port: 30140,
    portHolder: null,
};

/** The folders of `D:\FXServer`, as the listing gives them. */
const FXSERVER_FOLDERS: FolderListing = {
    path: 'D:\\FXServer\\',
    parent: 'D:\\',
    problem: null,
    pathProblem: null,
    entries: [
        {
            name: 'server-data',
            path: DATA_PATH,
            kind: 'server',
            link: false,
            resources: 54,
            cfg: 'server.cfg',
            folders: null,
            inside: [],
        },
        {
            name: 'build',
            path: 'D:\\FXServer\\build',
            kind: 'folder',
            link: false,
            resources: null,
            cfg: null,
            folders: 2,
            inside: [],
        },
    ],
    truncated: false,
};

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
    const field = () => view.getByRole('combobox', { name: words.folder.label }) as HTMLInputElement;
    /** Types a path in the folder field, then Enter checks it. */
    const typeFolder = async (path: string) => {
        fireEvent.change(field(), { target: { value: path } });
        fireEvent.keyDown(field(), { key: 'Enter' });
        await settle();
    };
    /** Through the folder step: Continue checks the path typed, and goes on once it passes. */
    const toCfg = async () => {
        fireEvent.change(field(), { target: { value: DATA_PATH } });
        await press(words.continue);
    };
    return { ...view, client, say, button, press, field, typeFolder, toCfg };
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

    beforeEach(() => {
        // The iris behind the setup stays still, as under reduced motion: jsdom draws no canvas.
        vi.stubGlobal('matchMedia', () => ({
            matches: true,
            addEventListener: () => {},
            removeEventListener: () => {},
        }));
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        vi.restoreAllMocks();
    });

    it('should check the folder, take the cfg found there, and check it on entering its step', async () => {
        const live: FolderReport = {
            ...FOLDER,
            cfgFiles: ['configs/live.cfg'],
            detectedCfg: 'configs/live.cfg',
        };
        const api = serve({ [FOLDER_ROUTE]: json(live), [CFG_ROUTE]: json(CFG) });
        const { button, getByRole, toCfg } = await page();

        expect(button(words.continue).disabled).toBe(true);
        await toCfg();

        expect(api.of(FOLDER_ROUTE)).toEqual([{ dataPath: DATA_PATH }]);
        expect(api.of(CFG_ROUTE)).toEqual([
            { dataPath: DATA_PATH, cfgFile: 'configs/live.cfg', onesync: 'on' },
        ]);
        expect(document.activeElement).toBe(getByRole('heading', { name: words.headings.cfg }));
        const steps = within(getByRole('list', { name: words.steps })).getAllByRole('listitem');
        expect(steps[1]?.getAttribute('aria-current')).toBe('step');
        expect(steps[0]?.textContent).toContain(words.said.passed);
    });

    it('should take the cfg detected in the folder checked last, not one detected before', async () => {
        const other: FolderReport = { ...FOLDER, cfgFiles: ['main.cfg'], detectedCfg: 'main.cfg' };
        const api = serve({ [FOLDER_ROUTE]: [json(other), json(FOLDER)], [CFG_ROUTE]: json(CFG) });
        const { press, typeFolder } = await page();

        await typeFolder('D:\\FXServer\\other-data');
        await typeFolder(DATA_PATH);
        await press(words.continue);

        expect(api.of(CFG_ROUTE)).toEqual([{ dataPath: DATA_PATH, cfgFile: 'server.cfg', onesync: 'on' }]);
    });

    it('should give the resources counted as a floor when the walk stopped at either of its limits', async () => {
        const many: FolderReport = {
            ...FOLDER,
            resources: { resources: 1800, categories: 250, withoutManifest: [], truncated: true },
        };
        serve({ [FOLDER_ROUTE]: json(many) });
        const { getByText, typeFolder } = await page();

        await typeFolder(DATA_PATH);

        getByText('1,800+');
        getByText(words.folder.countStopped);
    });

    it('should say why a folder is refused, stay on it at Continue, and offer the one nearby that fits', async () => {
        const resources = `${DATA_PATH}\\resources`;
        const refused: FolderReport = {
            ...FOLDER,
            problem: 'isResources',
            suggestion: DATA_PATH,
            resources: null,
            cfgFiles: [],
            detectedCfg: null,
        };
        const api = serve({
            [FOLDER_ROUTE]: [json(refused), json(refused), json(FOLDER)],
            [CFG_ROUTE]: json(CFG),
        });
        const { getByRole, field, press, typeFolder } = await page();

        await typeFolder(resources);

        expect(field().getAttribute('aria-invalid')).toBe('true');
        expect(field().getAttribute('aria-describedby')).toContain('error');
        expect(described(field())).toBe(words.folder.problems.isResources);

        await press(words.continue);
        expect(api.of(CFG_ROUTE)).toEqual([]);
        getByRole('heading', { name: words.headings.folder });

        await press(words.folder.useSuggestion);
        expect(field().value).toBe(DATA_PATH);
        await press(words.continue);

        expect(api.of(FOLDER_ROUTE)).toEqual([
            { dataPath: resources },
            { dataPath: resources },
            { dataPath: DATA_PATH },
        ]);
        expect(api.of(CFG_ROUTE)).toHaveLength(1);
    });

    it('should say a path is needed when a field is checked empty', async () => {
        const unread: CfgReport = {
            ...CFG,
            problem: 'invalidPath',
            pathProblem: 'required',
            files: [],
            facts: null,
            findings: [],
        };
        const api = serve({
            [FOLDER_ROUTE]: [
                json({ ...FOLDER, problem: 'invalidPath', pathProblem: 'required', resources: null }),
                json(FOLDER),
            ],
            [CFG_ROUTE]: [json(CFG), json(unread)],
        });
        const { getByRole, field, press, toCfg } = await page();

        fireEvent.keyDown(field(), { key: 'Enter' });
        await settle();
        expect(api.of(FOLDER_ROUTE)).toEqual([{ dataPath: '' }]);
        expect(described(field())).toBe(en.server.problems.required);

        await toCfg();
        const cfg = getByRole('textbox', { name: words.cfg.label });
        fireEvent.change(cfg, { target: { value: '' } });
        await press(words.cfg.checkAgain);
        expect(api.of(CFG_ROUTE).at(-1)).toEqual({ dataPath: DATA_PATH, cfgFile: '', onesync: 'on' });
        expect(described(cfg)).toBe(en.server.problems.required);
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

    it('should list a finding written twice on one line twice, and as often as the next check says', async () => {
        const monitor = finding('startsMonitor', 'warning', 9, { name: 'monitor' });
        const twice: CfgReport = { ...CFG, findings: [monitor, monitor] };
        const once: CfgReport = { ...CFG, findings: [monitor] };
        const errors = vi.spyOn(console, 'error').mockImplementation(() => {});
        serve({ [FOLDER_ROUTE]: json(FOLDER), [CFG_ROUTE]: [json(twice), json(once)] });
        const { getByRole, press, toCfg } = await page();
        const rows = (count: number) =>
            within(
                getByRole('heading', { name: words.cfg.groups.warning(count) }).closest(
                    'section',
                ) as HTMLElement,
            ).getAllByRole('listitem');

        await toCfg();
        expect(rows(2)).toHaveLength(2);
        await press(words.cfg.checkAgain);

        expect(rows(1)).toHaveLength(1);
        expect(errors).not.toHaveBeenCalled();
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

    it('should check the cfg again when the owner picks another OneSync, and count what it finds', async () => {
        const needsOnesync: CfgReport = {
            ...CFG,
            findings: [...CFG.findings, finding('maxClientsNeedOneSync', 'warning', 3, { count: 48 })],
        };
        const api = serve({
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: (body) =>
                Response.json((body as { onesync: string }).onesync === 'off' ? needsOnesync : CFG),
        });
        const { getByRole, getByText, press, toCfg } = await page();
        await toCfg();
        await press(words.continue);
        await press(words.continue);
        getByText(words.start.findingCount(2, 2));

        fireEvent.click(getByRole('radio', { name: en.server.onesyncModes.off }));
        await settle();

        expect(api.of(CFG_ROUTE).map((body) => (body as { onesync: string }).onesync)).toEqual(['on', 'off']);
        getByText(words.start.findingCount(3, 2));
    });

    it('should keep the report of the OneSync picked last when an earlier check answers after it', async () => {
        const needsOnesync: CfgReport = {
            ...CFG,
            findings: [...CFG.findings, finding('maxClientsNeedOneSync', 'warning', 3, { count: 48 })],
        };
        let answerOff: ((response: Response) => void) | undefined;
        serve({
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: (body) =>
                (body as { onesync: string }).onesync === 'off'
                    ? new Promise((resolve) => {
                          answerOff = resolve;
                      })
                    : Response.json(CFG),
        });
        const { getByRole, getByText, press, toCfg } = await page();
        await toCfg();
        await press(words.continue);
        await press(words.continue);

        fireEvent.click(getByRole('radio', { name: en.server.onesyncModes.off }));
        await settle();
        fireEvent.click(getByRole('radio', { name: en.server.onesyncModes.legacy }));
        await settle();
        answerOff?.(Response.json(needsOnesync));
        await settle();

        expect(
            (getByRole('radio', { name: en.server.onesyncModes.legacy }) as HTMLInputElement).checked,
        ).toBe(true);
        getByText(words.start.findingCount(2, 2));
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

        await waitFor(() => getByText(words.start.saved));
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
        await waitFor(() => getByRole('heading', { name: words.launch.starting }));

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
        getByRole('heading', { name: words.launch.online(STOPPED.name) });
        getByRole('link', { name: words.launch.overview });
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
        await waitFor(() => getByRole('heading', { name: words.launch.starting }));

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

    it('should keep a first launch that hung on the delay that ran out, with the way to raise it', async () => {
        serve({
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: json(CFG),
            [SAVE_ROUTE]: json(stored(1)),
            'POST /servers/default/start': () => new Response(null, { status: 204 }),
        });
        const { client, getByRole, getByText, press, say, toCfg } = await page();
        await toCfg();
        await press(words.continue);
        await press(words.continue);
        await press(words.start.saveAndStart);
        await waitFor(() => getByRole('heading', { name: words.launch.starting }));

        const run = { ...STOPPED, generation: 1, since: '2026-10-30T14:00:05Z' };
        say({
            type: 'event',
            topic: 'server.status',
            data: { ...run, state: 'hung', cause: { kind: 'hang', cause: 'noHeartbeat' } },
        });
        // Raised once the owner has read why: the delay that ran out is still the one said.
        const raised: StoredLaunch = {
            ...stored(2),
            settings: { ...stored(2).settings, hangTimeoutSeconds: 900 },
        };
        act(() => {
            client.setQueryData(launchQuery.queryKey, raised);
        });
        say({
            type: 'event',
            topic: 'server.status',
            data: { ...run, state: 'starting', generation: 2, cause: { kind: 'recovery' } },
        });

        const bridge = within(getByRole('list', { name: words.launch.title })).getAllByRole('listitem')[3];
        expect(bridge?.textContent).toContain(words.said.failed);
        getByText(en.server_state.hangNoHeartbeat);
        getByText(en.server_state.hangAdvice(6));
        expect(getByRole('link', { name: en.server_state.openHang }).getAttribute('href')).toBe(
            '/settings/launch#setting-hangDetection',
        );
    });

    it('should fail the first launch at its start when FXServer was tried and did not start', async () => {
        serve({
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: json(CFG),
            [SAVE_ROUTE]: json(stored(1)),
            'POST /servers/default/start': () =>
                Response.json({ code: 'launch_failed', params: { reason: 'missingData' } }, { status: 422 }),
        });
        const { getByRole, press, say, toCfg } = await page();
        await toCfg();
        await press(words.continue);
        await press(words.continue);

        await press(words.start.saveAndStart);
        await waitFor(() => getByRole('list', { name: words.launch.title }));
        say({
            type: 'event',
            topic: 'server.status',
            data: {
                ...STOPPED,
                since: '2026-10-30T14:00:05Z',
                cause: { kind: 'spawnFailed', reason: 'missingData' },
            },
        });

        const started = within(getByRole('list', { name: words.launch.title })).getAllByRole('listitem')[0];
        expect(started?.textContent).toContain(words.said.failed);
        getByRole('link', { name: words.launch.openConsole });
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
        expect(document.activeElement).toBe(getByRole('heading', { name: words.headings.cfg }));
        expect(api.of(CFG_ROUTE)).toHaveLength(2);
        expect(api.of('POST /servers/default/start')).toEqual([]);
    });

    it('should send the owner to the launch settings when the saved extra arguments are refused', async () => {
        serve({
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: json(CFG),
            [SAVE_ROUTE]: () =>
                Response.json(
                    {
                        code: 'invalid_launch_settings',
                        params: { issues: [{ field: 'argumentName', index: 0, problem: 'reserved' }] },
                    },
                    { status: 422 },
                ),
        });
        const { getByRole, getByText, press, toCfg } = await page({ saved: stored(3) });
        await toCfg();
        await press(words.continue);
        await press(words.continue);

        await press(words.start.saveAndStart);

        await waitFor(() => getByText(words.start.argumentsRefused));
        expect(getByRole('link', { name: words.start.openServer }).getAttribute('href')).toBe(
            '/settings/launch',
        );
    });

    it('should start again from the settings saved elsewhere when its version is stale', async () => {
        serve({
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: json(CFG),
            [SAVE_ROUTE]: () => Response.json({ code: 'conflict' }, { status: 409 }),
            'GET /servers/default/launch': json(stored(4)),
        });
        const { getByRole, getByText, press, toCfg } = await page({ saved: stored(3) });
        await toCfg();
        await press(words.continue);
        await press(words.continue);

        await press(words.start.saveAndStart);

        await waitFor(() => getByText(words.changedElsewhere));
        await press(words.startAgain);
        expect(document.activeElement).toBe(getByRole('heading', { name: words.headings.folder }));
    });

    it('should offer the servers found, the first one picked, check each one picked, and go on with the last', async () => {
        const api = serve({
            [DISCOVER_ROUTE]: json({ places: PLACES, servers: [LIVE, LAB], truncated: false }),
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: json(CFG),
        });
        const { getByRole, getByText, press } = await page();

        await waitFor(() => getByText(words.folder.found(2)));
        const found = within(getByRole('list', { name: words.folder.servers }));
        const live = found.getByRole('button', { name: /server-data/ });
        expect(live.getAttribute('aria-pressed')).toBe('true');
        within(live).getByText(words.folder.usedByTxAdmin);
        within(live).getByText(words.folder.onesync.on);
        getByText(words.folder.portTaken(30120, { program: 'FXServer.exe', fxServer: true }, true));
        within(live).getByText(words.folder.portInUse(30120, 'FXServer.exe'));
        await waitFor(() => getByText(words.folder.accepted));

        fireEvent.click(found.getByRole('button', { name: /lab-server/ }));
        await settle();
        expect(found.getByRole('button', { name: /lab-server/ }).getAttribute('aria-pressed')).toBe('true');
        fireEvent.click(found.getByRole('button', { name: /server-data/ }));
        await settle();
        await press(words.folder.continueWith('server-data'));

        expect(api.of(FOLDER_ROUTE)).toEqual([
            { dataPath: DATA_PATH },
            { dataPath: LAB.path },
            { dataPath: DATA_PATH },
        ]);
        expect(api.of(CFG_ROUTE)).toEqual([{ dataPath: DATA_PATH, cfgFile: 'server.cfg', onesync: 'on' }]);
        expect(document.activeElement).toBe(getByRole('heading', { name: words.headings.cfg }));
    });

    it('should browse to a folder, show what its check finds, and go on with it', async () => {
        const api = serve({
            [FOLDERS_ROUTE]: json(FXSERVER_FOLDERS),
            [FOLDER_ROUTE]: json(FOLDER),
            [CFG_ROUTE]: json(CFG),
        });
        const { getByRole, getByText, field, press } = await page();

        fireEvent.change(field(), { target: { value: 'D:\\FXServer\\build' } });
        await press(words.folder.browse);
        const folders = within(
            await waitFor(() => getByRole('list', { name: words.folder.browser.folders('FXServer') })),
        );
        fireEvent.click(folders.getByRole('button', { name: /^server-data/ }));

        await waitFor(() => getByText(words.folder.accepted));
        expect(api.of(FOLDERS_ROUTE)).toEqual([{ path: 'D:\\FXServer\\', prefix: '' }]);
        await press(words.folder.browser.use);

        expect(api.of(FOLDER_ROUTE).at(-1)).toEqual({ dataPath: DATA_PATH });
        expect(api.of(CFG_ROUTE)).toEqual([{ dataPath: DATA_PATH, cfgFile: 'server.cfg', onesync: 'on' }]);
    });
});
