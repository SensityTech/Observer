import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import { catalogQuery, cleanupQuery } from '../api/artifacts';
import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import type { CatalogEntry } from '../api/generated/CatalogEntry';
import type { CatalogView } from '../api/generated/CatalogView';
import type { InstalledBuild } from '../api/generated/InstalledBuild';
import { ArtifactsProvider } from '../artifacts/context';
import { ARTIFACTS } from '../artifacts/fixtures';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { ScheduleProvider } from '../schedule/context';
import { ServerStatusProvider } from '../server/context';
import { StatusBar } from '../shell/StatusBar';
import { ArtifactsPage } from './ArtifactsPage';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const FIXTURE = structuredClone(ARTIFACTS) as unknown as ArtifactsStatus;
// 35245 healthy and whole, as a build goes back to.
const STATUS: ArtifactsStatus = {
    ...FIXTURE,
    installed: FIXTURE.installed.map((build) => ({ ...build, health: { kind: 'valid' } })),
};
const BUILD: InstalledBuild = STATUS.installed[0] ?? FIXTURE.installed[0]!;

const ONLINE = {
    state: 'online',
    cause: { kind: 'ready' },
    since: '2026-10-28T02:00:00Z',
    startedAt: '2026-10-28T02:00:00Z',
    generation: 1,
    problems: [],
    build: 37900,
    players: { online: 187, max: 2048 },
    restartAt: null,
    launch: 'ready',
};

const SCHEDULE = {
    settings: {
        enabled: true,
        mode: 'fixedTimes',
        times: ['04:00'],
        intervalMinutes: 360,
        timeZone: 'Europe/Paris',
        warnings: [30, 15, 10, 5, 4, 3, 2, 1],
        skipAfterStart: 120,
    },
    version: 1,
    zone: 'Europe/Paris',
    next: { at: '2026-10-29T03:00:00Z', source: 'schedule', by: null, delayedFrom: null },
    upcoming: [],
    oneOff: null,
};

const entry = (build: number, more: Partial<CatalogEntry> = {}): CatalogEntry => ({
    build,
    commit: '0cfcdb5ead57a4a26fcb6ebad98adbcd0273e607',
    branch: 'master',
    recommended: false,
    latest: false,
    publishedAt: '2026-10-26T12:24:47Z',
    endOfSupport: { kind: 'never' },
    endOfLife: { kind: 'at', at: '2027-01-26T12:08:14Z' },
    support: 'supported',
    availability: { kind: 'downloadable', size: 66631791 },
    listed: true,
    installed: false,
    ...more,
});

const CATALOG: CatalogView = {
    platform: 'windows',
    fetchedAt: '2026-10-28T02:00:00Z',
    builds: [
        entry(37950, { latest: true }),
        entry(37900, { installed: true }),
        entry(35245, { recommended: true, installed: true }),
        entry(5848, {
            support: 'endOfLife',
            endOfSupport: { kind: 'at', at: '2024-01-01T00:00:00Z' },
            endOfLife: { kind: 'at', at: '2024-06-01T00:00:00Z' },
        }),
    ],
    warnings: [],
};

/** The page and the status bar, for the owner, with the builds as the topic says. */
const page = async (status: ArtifactsStatus = STATUS, permissions = 'fff') => {
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
    client.setQueryData(catalogQuery.queryKey, CATALOG);
    const view = render(
        <QueryClientProvider client={client}>
            <I18nProvider>
                <MeContext
                    value={{ id: 1, username: 'ada', isOwner: true, permissions, mustChangePassword: false }}
                >
                    <LiveContext value={live}>
                        <ServerStatusProvider>
                            <ScheduleProvider>
                                <ArtifactsProvider>
                                    <StatusBar />
                                    <ArtifactsPage />
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
    say({ type: 'snapshot', topic: 'server.status', data: ONLINE });
    say({ type: 'snapshot', topic: 'server.schedule', data: SCHEDULE });
    say({ type: 'snapshot', topic: 'server.artifacts', data: status });
    return { ...view, client, say, drop: () => act(() => socket?.close(1006)) };
};

/** Whether the Available builds card offers the install of `build`. */
const offered = (build: number) => {
    const available = [...document.querySelectorAll('section')].find((each) =>
        each.textContent?.startsWith(en.artifacts.available.title),
    ) as HTMLElement;
    const line = [...available.querySelectorAll('tbody tr')].find((each) =>
        each.textContent?.startsWith(String(build)),
    ) as HTMLElement;
    return within(line).queryByRole('button', { name: en.artifacts.available.install }) !== null;
};

const row = (build: number) => {
    const cell = [...document.querySelectorAll('td')].find((each) =>
        each.textContent?.startsWith(String(build)),
    );
    return cell?.closest('tr') as HTMLTableRowElement;
};

describe('ArtifactsPage', () => {
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

    it('should show each step of the running job, the current one marked, with what is downloaded', async () => {
        const { getByRole } = await page();

        const steps = within(getByRole('list', { name: en.artifacts.job.title })).getAllByRole('listitem');
        expect(steps.map((step) => step.textContent)).toEqual([
            'Checking',
            'Downloading',
            'Verifying',
            'Extracting',
            'Validating',
            'Installing',
        ]);
        expect(steps[1]?.getAttribute('aria-current')).toBe('step');
        expect(steps.filter((step) => step.hasAttribute('aria-current'))).toHaveLength(1);
        const progress = getByRole('progressbar', { name: en.artifacts.job.progress });
        expect(progress.getAttribute('value')).toBe('65');
        expect(progress.getAttribute('aria-valuetext')).toContain('39.3 MB of 60.6 MB');
    });

    it('should count the files once the download is over, without its figures', async () => {
        const job = STATUS.job ?? FIXTURE.job!;
        const { getByText, queryByText } = await page({
            ...STATUS,
            job: {
                ...job,
                step: 'extract',
                download: { received: 63_500_000, total: 63_500_000, rate: 4_200_000 },
                entries: { done: 120, total: 563 },
            },
        });

        expect(getByText('120 of 563 files')).toBeTruthy();
        expect(queryByText(/60\.6 MB of 60\.6 MB/)).toBeNull();
        expect(queryByText(/\/s/)).toBeNull();
    });

    it('should gate an install past its end of life behind its checkbox', async () => {
        const { getByLabelText, getByRole } = await page({ ...STATUS, job: null });
        fireEvent.click(getByLabelText(en.artifacts.available.showEndOfLife));
        fireEvent.click(within(row(5848)).getByRole('button', { name: en.artifacts.available.install }));

        const dialog = getByRole('dialog', { name: en.artifacts.install.title('5848') });
        const install = within(dialog).getByRole('button', { name: en.artifacts.install.confirm });
        expect((install as HTMLButtonElement).disabled).toBe(true);
        fireEvent.click(within(dialog).getByLabelText(en.artifacts.install.endOfLife));
        expect((install as HTMLButtonElement).disabled).toBe(false);
    });

    it('should apply a build at the scheduled restart by default when FXServer runs and one is ahead', async () => {
        const { getByRole } = await page({ ...STATUS, job: null });

        fireEvent.click(within(row(35245)).getByRole('button', { name: en.artifacts.installed.use }));

        const dialog = getByRole('dialog', { name: en.artifacts.use.title('35245') });
        const scheduled = within(dialog).getByRole('radio', { name: new RegExp(en.artifacts.use.scheduled) });
        const now = within(dialog).getByRole('radio', { name: new RegExp(en.artifacts.use.now) });
        expect((scheduled as HTMLInputElement).checked).toBe(true);
        expect((now as HTMLInputElement).checked).toBe(false);
        expect(dialog.textContent).toContain(en.artifacts.use.nowPlayers(187));
        expect(dialog.textContent).toContain(en.artifacts.use.replaces('37950'));
    });

    it('should apply at the next start by default when FXServer is stopped, never now', async () => {
        const { getByRole, say } = await page({ ...STATUS, job: null, running: null });
        say({
            type: 'event',
            topic: 'server.status',
            data: { ...ONLINE, state: 'stopped', startedAt: null, build: null, players: null },
        });

        fireEvent.click(within(row(35245)).getByRole('button', { name: en.artifacts.installed.use }));

        const dialog = getByRole('dialog', { name: en.artifacts.use.title('35245') });
        const next = within(dialog).getByRole('radio', { name: new RegExp(en.artifacts.use.nextStart) });
        const now = within(dialog).getByRole('radio', { name: new RegExp(en.artifacts.use.now) });
        expect((next as HTMLInputElement).checked).toBe(true);
        expect((now as HTMLInputElement).disabled).toBe(true);
        expect(dialog.textContent).toContain(en.artifacts.use.notRunning);
    });

    it('should ask for 6449 typed before going below it after a newer build ran', async () => {
        const imported: InstalledBuild = { ...BUILD, build: 6448, commit: null, origin: 'import' };
        const old: ArtifactsStatus = { ...STATUS, job: null, installed: [...STATUS.installed, imported] };
        const { getByRole } = await page(old);

        fireEvent.click(within(row(6448)).getByRole('button', { name: en.artifacts.installed.use }));

        const dialog = getByRole('dialog', { name: en.artifacts.use.title('6448') });
        const confirm = within(dialog).getByRole('button', {
            name: en.artifacts.use.confirm,
        }) as HTMLButtonElement;
        expect(dialog.textContent).toContain(en.artifacts.use.kvp);
        expect(confirm.disabled).toBe(true);
        fireEvent.change(within(dialog).getByLabelText(en.artifacts.use.kvpLabel), {
            target: { value: '6448' },
        });
        expect(confirm.disabled).toBe(true);
        fireEvent.change(within(dialog).getByLabelText(en.artifacts.use.kvpLabel), {
            target: { value: '6449' },
        });
        expect(confirm.disabled).toBe(false);
    });

    it('should say why a build in use cannot be deleted', async () => {
        const { getAllByRole } = await page({ ...STATUS, job: null });

        const remove = within(row(37900)).getByRole('button', { name: en.artifacts.installed.delete });
        fireEvent.click(remove);

        expect(remove.getAttribute('aria-disabled')).toBe('true');
        expect(remove.getAttribute('title')).toBe(en.artifacts.installed.protectedBy.selected);
        expect(
            getAllByRole('alert').some(
                (alert) => alert.textContent === en.artifacts.installed.protectedBy.selected,
            ),
        ).toBe(true);
    });

    it('should list in the cleanup what it removes and what it frees', async () => {
        const { client, getByRole } = await page({ ...STATUS, job: null });
        client.setQueryData(cleanupQuery.queryKey, {
            remove: [
                { build: 37500, bytes: 432013312, reinstallable: false },
                { build: 37400, bytes: 432013312, reinstallable: true },
            ],
            keep: [{ build: 37900, protected: ['selected', 'running'] }],
            freed: 864026624,
        });

        fireEvent.click(getByRole('button', { name: en.artifacts.installed.cleanup }));

        const dialog = getByRole('alertdialog', { name: en.artifacts.cleanup.title });
        const items = within(dialog)
            .getAllByRole('listitem')
            .map((item) => item.textContent);
        expect(items).toEqual([`37500412 MB · ${en.artifacts.cleanup.gone}`, '37400412 MB']);
        expect(dialog.textContent).toContain(en.artifacts.cleanup.freed('824 MB'));
        expect(within(dialog).getByRole('button', { name: en.artifacts.cleanup.confirm(2) })).toBeTruthy();
    });

    it('should say under the folder field why Observer cannot import from it', async () => {
        const refusal = { code: 'invalid_import_folder', params: { problem: 'notAbsolute' } };
        vi.stubGlobal(
            'fetch',
            vi.fn(async () => Response.json(refusal, { status: 400 })),
        );
        const { getByRole } = await page({ ...STATUS, job: null });
        fireEvent.click(getByRole('button', { name: en.artifacts.installed.importBuild }));
        const dialog = getByRole('dialog', { name: en.artifacts.import.title });

        fireEvent.change(within(dialog).getByLabelText(en.artifacts.import.folder), {
            target: { value: 'builds/35245' },
        });
        fireEvent.change(within(dialog).getByLabelText(en.artifacts.import.build), {
            target: { value: '35245' },
        });
        fireEvent.click(within(dialog).getByRole('button', { name: en.artifacts.import.confirm }));

        await waitFor(() => within(dialog).getByText(en.server.problems.notAbsolute));
        expect(within(dialog).queryByText(en.errors.invalid_request)).toBeNull();
    });

    it('should keep the times in the server zone, and say which, while the live link is down', async () => {
        const { container, getByText, drop } = await page({ ...STATUS, job: null });
        const head = container.querySelector('.page-head');

        // 02:28 UTC is 03:28 in Paris on 27 October.
        expect(head?.textContent).toContain('Times in Europe/Paris');
        expect(getByText(/since 27 Oct 03:28/)).toBeTruthy();
        drop();

        expect(getByText(en.artifacts.outOfDate)).toBeTruthy();
        expect(head?.textContent).toContain('Times in Europe/Paris');
        expect(getByText(/since 27 Oct 03:28/)).toBeTruthy();
    });

    it('should offer the install of a build by what the live status says is installed', async () => {
        // Installed since the catalog was read; and 35245, which the catalog says is installed,
        // deleted since.
        await page({
            ...STATUS,
            job: null,
            installed: [
                { ...BUILD, build: 37950 },
                ...STATUS.installed.filter((each) => each.build !== 35245),
            ],
        });

        expect(offered(37950)).toBe(false);
        expect(offered(35245)).toBe(true);
    });

    it('should put the focus on the way a build is applied by default, not on Restart now', async () => {
        const { getByRole } = await page({ ...STATUS, job: null });

        fireEvent.click(within(row(35245)).getByRole('button', { name: en.artifacts.installed.use }));

        const dialog = getByRole('dialog', { name: en.artifacts.use.title('35245') });
        const checked = within(dialog)
            .getAllByRole('radio')
            .find((each) => (each as HTMLInputElement).checked) as HTMLInputElement;
        expect(checked.value).toBe('scheduledRestart');
        expect(document.activeElement).toBe(checked);
    });

    it('should add the probation to the build in the status bar', async () => {
        const { container } = await page();

        const build = [...container.querySelectorAll('.status-cell')].find(
            (cell) => cell.querySelector('dt')?.textContent === en.status.build,
        );
        expect(build?.querySelector('dd')?.textContent).toBe(`37900 · ${en.status.probation}`);
    });
});
