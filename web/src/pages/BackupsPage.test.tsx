import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { BackupDestination } from '../api/generated/BackupDestination';
import type { BackupRun } from '../api/generated/BackupRun';
import type { BackupsView } from '../api/generated/BackupsView';
import type { Me } from '../api/generated/Me';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { BackupsPage } from './BackupsPage';

const words = en.backups;
const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const OWNER: Me = { id: 1, username: 'ada', isOwner: true, permissions: '3fffff', mustChangePassword: false };
/** BACKUPS_VIEW alone. */
const VIEWER: Me = {
    id: 2,
    username: 'kenji',
    isOwner: false,
    permissions: '80000',
    mustChangePassword: false,
};

const destination = (id: number, name: string, change: Partial<BackupDestination>): BackupDestination => ({
    id,
    name,
    enabled: true,
    target: { kind: 'local', folder: 'D:\\Backups\\sensity' },
    retention: { keepLast: 7, keepDays: null },
    publicKey: null,
    hasSecret: false,
    version: 1,
    createdAt: '2026-11-01T10:00:00Z',
    updatedAt: '2026-11-01T10:00:00Z',
    stored: 1,
    storedBytes: 412_000,
    last: { run: 41, state: 'stored', at: '2026-11-02T13:48:00Z', error: null, errorDetail: null },
    ...change,
});

const RUN: BackupRun = {
    id: 41,
    trigger: 'restart',
    actor: null,
    startedAt: '2026-11-02T13:47:00Z',
    finishedAt: '2026-11-02T13:47:38Z',
    outcome: 'partial',
    engine: 'mariaDb',
    database: 'sensity_main',
    host: '127.0.0.1:3306',
    client: 'mariadb-dump 11.7.2-MariaDB',
    fileName: 'sensity_main-20261102T134700Z-41.sql.gz',
    sqlBytes: 4_000_000_000,
    fileBytes: 432_013_312,
    fileSha256: 'a'.repeat(64),
    serverStarted: false,
    looseTables: [],
    looseTableCount: 0,
    error: null,
    errorDetail: null,
    copies: [
        {
            id: 101,
            destinationId: 1,
            destination: 'This machine',
            kind: 'local',
            state: 'stored',
            location: 'D:\\Backups\\sensity\\sensity_main-20261102T134700Z-41.sql.gz',
            bytes: 432_013_312,
            startedAt: '2026-11-02T13:47:39Z',
            storedAt: '2026-11-02T13:47:45Z',
            deletedAt: null,
            deleteReason: null,
            error: null,
            errorDetail: null,
        },
        {
            id: 102,
            destinationId: 2,
            destination: 'Backblaze B2',
            kind: 's3',
            state: 'stored',
            location: 'main/sensity_main-20261102T134700Z-41.sql.gz',
            bytes: 432_013_312,
            startedAt: '2026-11-02T13:47:46Z',
            storedAt: '2026-11-02T13:48:00Z',
            deletedAt: null,
            deleteReason: null,
            error: null,
            errorDetail: null,
        },
        {
            id: 103,
            destinationId: 3,
            destination: 'Storage box',
            kind: 'sftp',
            state: 'failed',
            location: 'sensity/sensity_main-20261102T134700Z-41.sql.gz',
            bytes: null,
            startedAt: '2026-11-02T13:48:01Z',
            storedAt: null,
            deletedAt: null,
            deleteReason: null,
            error: 'authentication',
            errorDetail: 'the server refused the key for u412',
        },
    ],
};

const VIEW: BackupsView = {
    settings: {
        beforeRestarts: 'scheduled',
        times: ['03:00'],
        restartWaitSeconds: 300,
        engine: 'mariaDb',
        source: 'cfg',
        manual: null,
        clientFolder: null,
    },
    version: 3,
    hasPassword: false,
    zone: 'UTC',
    nextAt: '2026-11-03T03:00:00Z',
    nextRestartAt: '2026-11-02T18:00:00Z',
    runsLast30Days: 41,
    source: {
        kind: 'cfg',
        host: '127.0.0.1',
        port: 3306,
        socket: null,
        user: 'fivem',
        database: 'sensity_main',
        problem: null,
    },
    client: {
        path: 'C:\\Program Files\\MariaDB 11.7\\bin\\mariadb-dump.exe',
        flavour: 'mariaDb',
        version: '11.7.2-MariaDB',
        problem: null,
    },
    destinations: [
        destination(1, 'This machine', {}),
        destination(2, 'Backblaze B2', {
            target: {
                kind: 's3',
                endpoint: 'https://s3.eu-central-003.backblazeb2.com',
                region: 'eu-central-003',
                bucket: 'sensity-backups',
                prefix: 'main/',
                accessKeyId: '003a9f1c2e7b0d40000000012',
                pathStyle: true,
            },
            retention: { keepLast: null, keepDays: 30 },
            hasSecret: true,
        }),
        destination(3, 'Storage box', {
            target: {
                kind: 'sftp',
                host: 'u412.your-storagebox.de',
                port: 23,
                user: 'u412',
                folder: 'sensity',
                auth: 'key',
                hostKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIKey',
            },
            publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIObserver observer-backups',
            retention: { keepLast: 14, keepDays: null },
            last: {
                run: 41,
                state: 'failed',
                at: '2026-11-02T13:48:01Z',
                error: 'authentication',
                errorDetail: 'the server refused the key for u412',
            },
        }),
    ],
    running: null,
    recent: [RUN],
};

const answer = (status: number, body?: unknown) =>
    new Response(body === undefined ? null : JSON.stringify(body), {
        status: body === undefined && status === 200 ? 204 : status,
        headers: { 'content-type': 'application/json' },
    });

/** Observer's answers by method and route; every request made is kept, with its body. */
const serve = (replies: Record<string, unknown>) => {
    const asked: { route: string; body: unknown }[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string, init: RequestInit) => {
            const url = new URL(input, 'http://localhost');
            const route = `${init.method ?? 'GET'} ${url.pathname.replace(/^\/api/, '')}`;
            asked.push({ route, body: init.body === undefined ? undefined : JSON.parse(String(init.body)) });
            const reply = replies[route];
            return reply === undefined ? answer(404, { code: 'not_found' }) : answer(200, reply);
        }),
    );
    return asked;
};

const SERVED = {
    'GET /servers/default/backups': VIEW,
    'GET /servers/default/backups/runs': { runs: [RUN], nextBefore: null },
};

const page = async (me: Me) => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const view = render(
        <I18nProvider>
            <QueryClientProvider client={client}>
                <MeContext value={me}>
                    <BackupsPage />
                </MeContext>
            </QueryClientProvider>
        </I18nProvider>,
    );
    await settle();
    await settle();
    return view;
};

describe('BackupsPage', () => {
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

    it('should say how the last backup went, what comes next and how each destination fares', async () => {
        serve(SERVED);
        const { getByText, getAllByText, getAllByRole, findByText, container } = await page(OWNER);

        expect(container.textContent).toMatch(/Last backup .*13:47, 412 MB, copied to 2 of 3 destinations\./);
        expect(container.textContent).toMatch(/Next one before the .*18:00 restart\./);
        expect(getByText(words.when.scheduled)).toBeTruthy();
        expect(getByText(words.when.waits('5 min'))).toBeTruthy();
        expect(getByText('03:00')).toBeTruthy();
        expect(getByText('sensity_main')).toBeTruthy();
        expect(getByText('· with mariadb-dump 11.7.2-MariaDB')).toBeTruthy();
        expect(getByText('Keeps 30 days')).toBeTruthy();
        expect(getAllByText(/^OK · .*13:48$/)).toHaveLength(2);
        expect(getByText('Failed · login refused')).toBeTruthy();
        expect(getAllByRole('button', { name: words.destinations.fix })).toHaveLength(1);
        const row = (await findByText('Before restart')).closest('tr');
        expect(row).not.toBeNull();
        expect(within(row as HTMLElement).getByText('2 of 3')).toBeTruthy();
        expect(
            within(row as HTMLElement).getByRole('button', { name: /Download the backup of/ }),
        ).toBeTruthy();
        expect(getByText(words.history.meta(41))).toBeTruthy();
    });

    it('should leave out what the account may not do', async () => {
        serve(SERVED);
        const { queryByRole } = await page(VIEWER);

        expect(queryByRole('button', { name: words.backUpNow })).toBeNull();
        expect(queryByRole('button', { name: words.when.edit })).toBeNull();
        expect(queryByRole('button', { name: words.destinations.add })).toBeNull();
        expect(queryByRole('button', { name: words.destinations.fix })).toBeNull();
        expect(queryByRole('button', { name: /Download the backup of/ })).toBeNull();
    });

    it('should start a backup when asked', async () => {
        const asked = serve({ ...SERVED, 'POST /servers/default/backups/run': RUN });
        const { getByRole } = await page(OWNER);

        fireEvent.click(getByRole('button', { name: words.backUpNow }));
        await settle();

        expect(asked.map((request) => request.route)).toContain('POST /servers/default/backups/run');
    });

    it('should add an S3 destination with the region and addressing its endpoint implies', async () => {
        const asked = serve({
            ...SERVED,
            'POST /servers/default/backups/destinations': VIEW.destinations[1],
        });
        const { getByRole, getByLabelText } = await page(OWNER);

        fireEvent.click(getByRole('button', { name: words.destinations.add }));
        fireEvent.click(getByLabelText(new RegExp(words.destinations.kinds.s3)));
        const dialog = within(getByRole('dialog'));
        fireEvent.change(dialog.getByLabelText(words.dialog.name), { target: { value: 'Backblaze B2' } });
        fireEvent.change(dialog.getByLabelText(words.dialog.endpoint), {
            target: { value: 'https://s3.eu-central-003.backblazeb2.com' },
        });
        fireEvent.change(dialog.getByLabelText(words.dialog.bucket), {
            target: { value: 'sensity-backups' },
        });
        fireEvent.change(dialog.getByLabelText(words.dialog.prefix), { target: { value: 'main' } });
        fireEvent.change(dialog.getByLabelText(words.dialog.accessKey), { target: { value: '003a9f1c' } });
        fireEvent.change(dialog.getByLabelText(words.dialog.secretKey), { target: { value: 'k3y' } });
        fireEvent.click(dialog.getByRole('button', { name: words.dialog.add }));
        await settle();

        const sent = asked.find((request) => request.route === 'POST /servers/default/backups/destinations');
        expect(sent?.body).toEqual({
            name: 'Backblaze B2',
            enabled: true,
            target: {
                kind: 's3',
                endpoint: 'https://s3.eu-central-003.backblazeb2.com',
                region: 'eu-central-003',
                bucket: 'sensity-backups',
                prefix: 'main/',
                accessKeyId: '003a9f1c',
                pathStyle: true,
            },
            retention: { keepLast: 14, keepDays: null },
            secret: 'k3y',
        });
    });

    it("should show Observer's key once an SFTP destination that logs in with it is added", async () => {
        const added = destination(4, 'Offsite', {
            target: {
                kind: 'sftp',
                host: 'backup.example.net',
                port: 22,
                user: 'fivem',
                folder: '',
                auth: 'key',
                hostKey: null,
            },
            publicKey: 'ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINew observer-backups',
            last: null,
        });
        serve({ ...SERVED, 'POST /servers/default/backups/destinations': added });
        const { getByRole, getByLabelText } = await page(OWNER);

        fireEvent.click(getByRole('button', { name: words.destinations.add }));
        fireEvent.click(getByLabelText(new RegExp(words.destinations.kinds.sftp)));
        const dialog = within(getByRole('dialog'));
        expect(dialog.getByRole('button', { name: words.dialog.test })).toHaveProperty('disabled', true);
        fireEvent.change(dialog.getByLabelText(words.dialog.name), { target: { value: 'Offsite' } });
        fireEvent.change(dialog.getByLabelText(words.dialog.host), {
            target: { value: 'backup.example.net' },
        });
        fireEvent.change(dialog.getByLabelText(words.dialog.user), { target: { value: 'fivem' } });
        fireEvent.click(dialog.getByRole('button', { name: words.dialog.add }));
        await settle();

        const after = within(getByRole('dialog'));
        expect(after.getByText(words.dialog.added)).toBeTruthy();
        expect(after.getByLabelText(words.dialog.publicKey)).toHaveProperty('value', added.publicKey);
        expect(after.getByRole('button', { name: words.dialog.test })).toHaveProperty('disabled', false);
    });
});
