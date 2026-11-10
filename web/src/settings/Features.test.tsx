import { cleanup, fireEvent, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { Feature } from '../api/generated/Feature';
import type { FeatureList } from '../api/generated/FeatureList';
import type { FeatureView } from '../api/generated/FeatureView';
import type { Me } from '../api/generated/Me';
import { en } from '../i18n/en';
import { openSettings } from './fixtures';

const words = en.settings.features;
const items = en.settings.items;

/** Holds View settings, and nothing else. */
const VIEWER: Me = {
    id: 2,
    username: 'kenji',
    isOwner: false,
    permissions: '200',
    mustChangePassword: false,
};

const view = (feature: Feature, change: Partial<FeatureView> = {}): FeatureView => ({
    feature,
    enabled: true,
    running: true,
    held: null,
    requires: [],
    turnOn: 'observerRestart',
    turnOff: 'now',
    changedAt: null,
    reach: {
        listens: [],
        tasks: [],
        whileOff: [],
        files: [],
        serverFolder: 'untouched',
        routes: [],
        topics: [],
    },
    ...change,
});

const LIST: FeatureList = {
    features: [
        view('sftp', {
            enabled: false,
            running: false,
            held: 'host',
            reach: {
                listens: ['0.0.0.0:40126'],
                tasks: [{ task: 'sftpServer', cadence: { kind: 'continuous' } }],
                whileOff: [{ task: 'uploadsSweep', cadence: { kind: 'observerStart' } }],
                files: [],
                serverFolder: 'writes',
                routes: [],
                topics: [],
            },
        }),
        view('changes'),
        view('tools', { enabled: false, running: false, turnOn: 'nextServerStart' }),
        view('diagnostics'),
        view('metrics', {
            reach: {
                listens: [],
                tasks: [
                    { task: 'sampler', cadence: { kind: 'every', seconds: 5 } },
                    { task: 'metricsUpkeep', cadence: { kind: 'every', seconds: 60 } },
                ],
                whileOff: [],
                files: [{ path: 'metrics.sqlite3', bytes: 640 * 1024 * 1024 }],
                serverFolder: 'untouched',
                routes: [],
                topics: [],
            },
        }),
        view('players', { enabled: false, running: false }),
        view('drops', { running: false, held: 'requires', requires: ['players'] }),
    ],
};

/** Observer's answers for the switches; every request made is kept, with its body. */
const serve = (answer: (body: { enabled: boolean }) => FeatureList | Response = () => LIST) => {
    const asked: { route: string; body: unknown }[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string, init: RequestInit) => {
            const route = `${init.method ?? 'GET'} ${String(input).replace(/^\/api/, '')}`;
            const body = typeof init.body === 'string' ? JSON.parse(init.body) : undefined;
            asked.push({ route, body });
            if (route === 'GET /features') {
                return Response.json(LIST);
            }
            if (route.startsWith('PUT /features/')) {
                const reply = answer(body);
                return reply instanceof Response ? reply : Response.json(reply);
            }
            return Response.json({ code: 'not_found', params: {} }, { status: 404 });
        }),
    );
    return asked;
};

/** What a route of SFTP answers while `observer.toml` gives it no address. */
const refused = () =>
    Response.json(
        { code: 'feature_disabled', params: { feature: 'sftp', held: 'notConfigured' } },
        { status: 403 },
    );

/** The card of a feature, by its name. */
const card = (feature: keyof typeof items) => {
    const section = document.getElementById(`setting-${feature}`);
    if (section === null) {
        throw new Error(`no card ${feature}`);
    }
    return within(section);
};

describe('Features settings', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should show each feature with what it does, what it costs, its state and when a switch applies', async () => {
        serve();
        await openSettings('/settings/features');

        const metrics = card('switchMetrics');
        expect(metrics.getByText(items.switchMetrics.about)).not.toBeNull();
        expect(metrics.getByText('every 5 s')).not.toBeNull();
        expect(metrics.getByText('every 1 min')).not.toBeNull();
        expect(metrics.getByText('metrics.sqlite3 · 640 MB')).not.toBeNull();
        expect(metrics.getByText(words.states.on)).not.toBeNull();
        expect(metrics.getByText(words.turnOff.now)).not.toBeNull();

        const tools = card('switchTools');
        expect(tools.getByText(words.states.off)).not.toBeNull();
        expect(tools.getByText(words.turnOn.nextServerStart)).not.toBeNull();
        expect(card('switchPlayers').getByText(words.turnOn.observerRestart)).not.toBeNull();
        expect(
            card('switchDrops').getByText(words.states.requires([items.switchPlayers.label])),
        ).not.toBeNull();
    });

    it('should not let a feature locked in observer.toml be switched, and say what it still finishes', async () => {
        serve();
        await openSettings('/settings/features');

        const sftp = card('switchSftp');
        expect(sftp.getByText(words.states.locked)).not.toBeNull();
        expect(
            (sftp.getByRole('switch', { name: items.switchSftp.label }) as HTMLInputElement).disabled,
        ).toBe(true);
        expect(sftp.getByText(words.listens('0.0.0.0:40126'))).not.toBeNull();
        expect(sftp.getByText(words.serverFolder.writes)).not.toBeNull();
        expect(sftp.getByText(words.whileOff([words.tasks.uploadsSweep]))).not.toBeNull();
    });

    it('should switch a feature off and say its tasks have stopped', async () => {
        const off: FeatureList = {
            features: LIST.features.map((each) =>
                each.feature === 'metrics' ? { ...each, enabled: false, running: false } : each,
            ),
        };
        const asked = serve(() => off);
        const { getByText } = await openSettings('/settings/features');

        fireEvent.click(card('switchMetrics').getByRole('switch', { name: items.switchMetrics.label }));

        await waitFor(() => getByText(words.turnedOff(items.switchMetrics.label)));
        expect(asked.find(({ route }) => route === 'PUT /features/metrics')?.body).toEqual({
            enabled: false,
        });
        expect(card('switchMetrics').getByText(words.states.off)).not.toBeNull();
    });

    it('should say a feature turned on runs only once Observer restarts, as the answer says', async () => {
        const on: FeatureList = {
            features: LIST.features.map((each) =>
                each.feature === 'players' ? { ...each, enabled: true } : each,
            ),
        };
        serve(() => on);
        const { getByText } = await openSettings('/settings/features');

        fireEvent.click(card('switchPlayers').getByRole('switch', { name: items.switchPlayers.label }));

        await waitFor(() =>
            getByText(words.turnedOnLater(items.switchPlayers.label, words.states.pending.observerRestart)),
        );
    });

    it('should say why a switch was refused', async () => {
        serve(() => Response.json({ code: 'feature_locked', params: { feature: 'tools' } }, { status: 403 }));
        const { findByText } = await openSettings('/settings/features');

        fireEvent.click(card('switchTools').getByRole('switch', { name: items.switchTools.label }));

        expect(await findByText(new RegExp(`^${items.switchTools.label}: `))).not.toBeNull();
    });

    it('should show the switches to an account that views settings, without letting it switch them', async () => {
        serve();
        const { getByText } = await openSettings('/settings/features', { me: VIEWER });

        expect(getByText(words.readOnly)).not.toBeNull();
        expect((card('switchMetrics').getByRole('switch') as HTMLInputElement).disabled).toBe(true);
    });

    it('should say SFTP waits for its address, and lead the owner to the form that opens its port', async () => {
        const address = en.settings.sftpAddress;
        const waiting: FeatureList = {
            features: [
                view('sftp', { running: false, held: 'notConfigured', turnOn: 'now' }),
                view('changes'),
            ],
        };
        const sent: unknown[] = [];
        vi.stubGlobal(
            'fetch',
            vi.fn(async (input: string, init: RequestInit) => {
                const route = `${init.method ?? 'GET'} ${String(input).replace(/^\/api/, '')}`;
                if (route === 'GET /features') {
                    return Response.json(waiting);
                }
                if (route === 'GET /servers/default/files/groups' || route === 'GET /files/accesses') {
                    return refused();
                }
                if (route === 'GET /settings/sftp') {
                    return Response.json({
                        listen: null,
                        pinned: false,
                        open: false,
                        fingerprint: null,
                        refused: null,
                    });
                }
                if (route === 'PUT /settings/sftp') {
                    sent.push(typeof init.body === 'string' ? JSON.parse(init.body) : undefined);
                    return Response.json({
                        listen: '0.0.0.0:40126',
                        pinned: false,
                        open: true,
                        fingerprint: 'SHA256:c29tZSBob3N0IGtleQ',
                        refused: null,
                    });
                }
                return Response.json({ code: 'not_found', params: {} }, { status: 404 });
            }),
        );
        const { findByText, findByRole, getByLabelText, queryByText } =
            await openSettings('/settings/features');

        const sftp = card('switchSftp');
        expect(sftp.getByText(words.states.notConfigured)).not.toBeNull();
        expect(sftp.getByText(address.waits)).not.toBeNull();
        expect(card('switchChanges').queryByText(address.waits)).toBeNull();
        fireEvent.click(sftp.getByRole('link', { name: address.give }));

        // The page it leads to holds the form, on the address and the port Observer suggests.
        expect(await findByText(address.none)).not.toBeNull();
        expect((getByLabelText(address.port) as HTMLInputElement).value).toBe('40126');
        expect(queryByText(en.files.intro)).toBeNull();
        expect(
            queryByText(en.errors.feature_disabled({ feature: 'sftp', held: 'notConfigured' })),
        ).toBeNull();
        fireEvent.click(await findByRole('button', { name: address.open }));

        expect(await findByText(address.listening('0.0.0.0:40126'))).not.toBeNull();
        expect(sent).toEqual([{ listen: '0.0.0.0:40126' }]);
        cleanup();

        // An account that only views settings reads what it waits for, without the way to a page it cannot open.
        await openSettings('/settings/features', { me: VIEWER });
        expect(card('switchSftp').getByText(address.waits)).not.toBeNull();
        expect(card('switchSftp').queryByRole('link', { name: address.give })).toBeNull();
    });

    it('should say the port of SFTP did not open, and where to see why', async () => {
        const address = en.settings.sftpAddress;
        vi.stubGlobal(
            'fetch',
            vi.fn(async () =>
                Response.json({
                    features: [view('sftp', { running: false, held: 'portRefused', turnOn: 'now' })],
                } satisfies FeatureList),
            ),
        );
        await openSettings('/settings/features');

        const sftp = card('switchSftp');
        expect(sftp.getByText(words.states.portRefused)).not.toBeNull();
        expect(sftp.getByText(address.refusedShort)).not.toBeNull();
        expect(sftp.getByRole('link', { name: address.see })).not.toBeNull();
    });

    it('should count the features left on beside their category', async () => {
        serve();
        const { getByRole } = await openSettings('/settings/features');

        const nav = within(getByRole('navigation', { name: en.settings.categoriesLabel }));
        expect(
            nav.getByRole('link', { name: new RegExp(en.settings.categories.features.name) }).textContent,
        ).toContain(words.count(4, 7));
    });
});
