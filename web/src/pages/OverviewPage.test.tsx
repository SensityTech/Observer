import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, render, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { ApiError } from '../api/client';
import type { LiveSample } from '../api/generated/LiveSample';
import type { Me } from '../api/generated/Me';
import type { ServerStatus } from '../api/generated/ServerStatus';
import type { StoredLaunch } from '../api/generated/StoredLaunch';
import type { MetricsSeries } from '../api/generated/MetricsSeries';
import type { RunGrowth } from '../api/generated/RunGrowth';
import type { ThreadShares } from '../api/generated/ThreadShares';
import { healthQuery } from '../api/health';
import { DEFAULT_LAUNCH, launchQuery } from '../api/launch';
import { MeContext } from '../auth/me';
import { growthQuery, seriesQuery, threadsQuery } from '../api/metrics';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { TILE_SERIES } from '../overview/Tiles';
import { ServerStatusProvider } from '../server/context';
import { OverviewPage } from './OverviewPage';

// The state card links to other pages; no router runs here.
vi.mock('@tanstack/react-router', () => ({
    Link: ({
        to,
        hash,
        children,
        className,
    }: {
        to: string;
        hash?: string;
        children: ReactNode;
        className?: string;
    }) => (
        <a href={hash === undefined ? to : `${to}#${hash}`} className={className}>
            {children}
        </a>
    ),
}));

const chart = vi.hoisted(() => ({ fails: false }));

// The charts draw on a canvas, which the test DOM does not have.
vi.mock('../charts/PlayersChart', () => ({
    default: () => {
        if (chart.fails) {
            throw new Error('the chart could not be drawn');
        }
        return null;
    },
}));
vi.mock('../charts/TickChart', () => ({ default: () => null }));

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const NOW = Math.floor(Date.now() / 5000) * 5;

const values = (at: (index: number) => number | null) => Array.from({ length: 720 }, (_, index) => at(index));

/** An hour with players in its last slots, a hitch ten minutes ago and a quarter of the CPU in the kernel. */
const hour = (): MetricsSeries => {
    return {
        range: '1h',
        step: 5,
        from: NOW - 3595,
        buckets: 720,
        last: NOW - 5,
        series: TILE_SERIES.map((series) => ({
            series,
            values: values((index) => {
                if (series === 'joins') return index === 700 ? 12 : 0;
                if (series === 'leaves') return index === 701 ? 9 : 0;
                if (series === 'hitchesMain') return index > 500 ? Number(index === 600) : null;
                if (series === 'hitchMax') return index === 600 ? 412 : null;
                if (series === 'fxsKernel') return index > 700 ? (10 + (index % 3)) / 4 : null;
                return index > 700 ? 10 + (index % 3) : null;
            }),
        })),
        markers: [],
        observed: Array.from({ length: 720 }, (_, index) => Number(index > 500)),
        online: Array.from({ length: 720 }, (_, index) => Number(index > 500)),
    };
};

/** An hour of which `kept` series were measured, without a hitch. */
const quietHour = (kept: readonly string[]): MetricsSeries => ({
    ...hour(),
    series: TILE_SERIES.map((series) => ({
        series,
        values: values((index) => (kept.includes(series) && index > 700 && series !== 'hitchMax' ? 0 : null)),
    })),
});

const SAMPLE: LiveSample = {
    ts: NOW,
    state: 'online',
    values: {
        players: 187,
        slots: 256,
        pingAvg: 41.2,
        pingP95: 88,
        mainTick: 6.84,
        mainOverBudget: 0.42,
        networkTick: 1.2,
        fxsCpu: 182.4,
        fxsKernel: 45.6,
        mainThreadCpu: 58,
        networkThreadCpu: 9,
        syncThreadCpu: 7,
        jsThreadsCpu: 12,
        monoThreadsCpu: 5,
        otherThreadsCpu: 3,
        treeCpu: 190,
        fxsFaults: 1200,
        hostClock: 4760,
        hostClockMax: 3400,
        hostClockLimit: 100,
        fxsThreads: 142,
        fxsHandles: 3210,
        children: 3,
        childrenCpu: 4.1,
        fxsPrivate: 9.4 * 1024 ** 3,
        fxsWorkingSet: 10.1 * 1024 ** 3,
        hostCpu: 41,
        hostMemoryUsed: 22.1 * 1024 ** 3,
        hostMemoryTotal: 64 * 1024 ** 3,
        hostReceive: 1_500_000,
        hostSend: 250_000,
        observerWorkingSet: 38 * 1024 ** 2,
        observerCpu: 0.4,
        dashboardClients: 2,
        resources: 54,
    },
    children: [],
    cores: 16,
};

/** The hour of `treeStarted`, measured on Windows only. */
const startedHour = (started: number | null): MetricsSeries => ({
    ...hour(),
    series: [{ series: 'treeStarted', values: values((index) => (index === 600 ? started : null)) }],
});

/** The last hour's threads of a process using `process`% of one core, by name. */
const shares = (process: number, threads: [string, number][]): ThreadShares => ({
    from: NOW - 3595,
    to: NOW + 5,
    measured: 720,
    process,
    threads: threads.map(([name, cpu]) => ({ name, group: name === '' ? 'other' : 'main', cpu })),
});

/** The page with Observer's health and the last hour already read, and a socket the test speaks for. */
const page = async (
    lastHour: MetricsSeries = hour(),
    {
        growth = null,
        started = null,
        threads = null,
        me = null,
    }: {
        growth?: RunGrowth | null;
        started?: number | null;
        threads?: ThreadShares | null;
        /** Signed in, with the server's state followed: the state card shows it. */
        me?: Me | null;
    } = {},
) => {
    const client = new QueryClient({
        defaultOptions: { queries: { staleTime: Infinity, retry: false, refetchInterval: false } },
    });
    client.setQueryData(healthQuery.queryKey, { status: 'ok', version: '0.1.0' });
    client.setQueryData(seriesQuery('1h', TILE_SERIES).queryKey, lastHour);
    client.setQueryData(seriesQuery('1h', ['treeStarted']).queryKey, startedHour(started));
    client.setQueryData(growthQuery.queryKey, growth);
    client.setQueryData(
        threadsQuery.queryKey,
        threads ?? { from: NOW - 3595, to: NOW + 5, measured: 0, process: null, threads: [] },
    );
    let socket: SocketEvents | undefined;
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: () => {}, close: () => {} };
        },
        refetch: () => {},
    });
    const launch: StoredLaunch = {
        settings: { ...DEFAULT_LAUNCH, dataPath: '/srv/fxserver/server-data', hangTimeoutSeconds: 900 },
        version: 1,
        legacyBuildPath: null,
    };
    client.setQueryData(launchQuery.queryKey, launch);
    const view = render(
        <QueryClientProvider client={client}>
            <I18nProvider>
                <LiveContext value={live}>
                    {me === null ? (
                        <OverviewPage />
                    ) : (
                        <MeContext value={me}>
                            <ServerStatusProvider>
                                <OverviewPage />
                            </ServerStatusProvider>
                        </MeContext>
                    )}
                </LiveContext>
            </I18nProvider>
        </QueryClientProvider>,
    );
    live.start();
    await settle();
    const say = (message: unknown) => act(() => socket?.message(JSON.stringify(message)));
    say({ type: 'hello', connectionId: 1, contract: 1 });
    say({
        type: 'snapshot',
        topic: 'status',
        data: { version: '0.1.0', startedAt: new Date(Date.now() - 300_000).toISOString() },
    });
    return { ...view, client, live, say, drop: () => act(() => socket?.close(1006)) };
};

/** The tile's figure with its unit, as it reads. */
const figure = (label: string) => document.querySelector(`section[aria-label="${label}"] p`)?.textContent;

const tile = (label: string) => {
    const section = document.querySelector<HTMLElement>(`section[aria-label="${label}"]`);
    if (section === null) {
        throw new Error(`no ${label} tile`);
    }
    return within(section);
};

/** A card of the second row, by its title. */
const card = (title: string) => {
    const heading = [...document.querySelectorAll('h2')].find((element) => element.textContent === title);
    const section = heading?.closest('section');
    if (section === null || section === undefined) {
        throw new Error(`no ${title} card`);
    }
    return within(section);
};

const OWNER: Me = { id: 1, username: 'ada', isOwner: true, permissions: 'fff', mustChangePassword: false };

/** The page for `me` while FXServer is declared hung for its silent HTTP endpoint. */
const hungFor = async (me: Me) => {
    // The tiles `me` may see read what the test does not serve.
    vi.stubGlobal(
        'fetch',
        vi.fn(async () => Response.json({ code: 'not_found' }, { status: 404 })),
    );
    const view = await page(hour(), { me });
    view.say({
        type: 'snapshot',
        topic: 'server.status',
        data: {
            state: 'hung',
            cause: { kind: 'hang', cause: 'noHttp' },
            since: new Date().toISOString(),
            startedAt: new Date(Date.now() - 600_000).toISOString(),
            generation: 1,
            problems: [],
            build: null,
            players: null,
            restartAt: null,
            launch: 'ready',
        } satisfies ServerStatus,
    });
    return view;
};

const lastHitch = () =>
    tile(en.tiles.tick.label).getByText(en.tiles.tick.lastHitch).nextElementSibling?.textContent;

describe('OverviewPage', () => {
    afterEach(() => {
        cleanup();
        chart.fails = false;
    });

    it('should keep the tiles when the chart fails, and offer a reload in its card', async () => {
        chart.fails = true;
        vi.spyOn(console, 'error').mockImplementation(() => {});
        const { say, live, findByText } = await page();
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: SAMPLE } });

        expect(await findByText(en.charts.players.unavailable)).not.toBeNull();
        expect(tile(en.tiles.players.label).queryByText('187')).not.toBeNull();
        expect(document.querySelector('button')?.textContent).toBe(en.common.reload);
        vi.restoreAllMocks();
        live.stop();
    });

    it('should fill every tile from the latest sample and the last hour', async () => {
        const { say, live } = await page();
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: SAMPLE } });

        const players = tile(en.tiles.players.label);
        expect(players.getByText('187').textContent).toBe('187/ 256');
        expect(players.getByText('41.2 ms · p95 88 ms')).not.toBeNull();
        expect(players.getByText('12 joined · 9 left')).not.toBeNull();
        expect(players.getByRole('img', { name: en.tiles.players.trend })).not.toBeNull();
        const tick = tile(en.tiles.tick.label);
        expect(tick.getByText('6.8').textContent).toBe('6.8ms avg');
        expect(tick.getByText('0.4%')).not.toBeNull();
        expect(tick.getByText('1.2 ms · n/a')).not.toBeNull();
        expect(tick.getByText(/^412 ms at \d\d:\d\d$/)).not.toBeNull();
        const cpu = tile(en.tiles.cpu.label);
        expect(cpu.getByText('182%').textContent).toBe('182% of one core');
        expect(cpu.getByText('11.4%')).not.toBeNull();
        expect(cpu.getByText('58%')).not.toBeNull();
        expect(cpu.getByText('9% · 7%')).not.toBeNull();
        expect(cpu.getByText('12% · 5% · 3%')).not.toBeNull();
        expect(cpu.getByText('user 75%')).not.toBeNull();
        expect(cpu.getByText('kernel 25%')).not.toBeNull();
        const tree = card(en.tree.title);
        expect(tree.getByText('142 · 3,210')).not.toBeNull();
        expect(tree.getByText('3 processes · 4.1%')).not.toBeNull();
        expect(tree.getByText('240/s')).not.toBeNull();
        expect(tree.getByText(en.tree.endedIncluded)).not.toBeNull();
        const memory = tile(en.tiles.memory.label);
        expect(memory.getByText('9.4').textContent).toBe('9.4GB private');
        expect(memory.getByText('10.1 GB')).not.toBeNull();
        expect(memory.getAllByText('n/a')).toHaveLength(3);
        expect(memory.queryByText(en.tiles.memory.possibleLeak)).toBeNull();
        const host = tile(en.tiles.host.label);
        expect(host.getByText('22.1 GB / 64 GB')).not.toBeNull();
        expect(host.getByText('12 Mbit/s · 2 Mbit/s')).not.toBeNull();
        expect(host.getByText('4.8 GHz · 3.4 GHz rated')).not.toBeNull();
        expect(host.queryByText(en.tiles.host.steal)).toBeNull();
        expect(host.queryByText(en.tiles.host.pressure)).toBeNull();
        const observer = tile(en.tiles.observer.label);
        expect(observer.getByText('38').textContent).toBe('38MB RAM');
        expect(observer.getByText(en.tiles.observer.running)).not.toBeNull();
        expect(observer.getByText(/^0\.1\.0 · up \d min$/)).not.toBeNull();
        live.stop();
    });

    it('should flag a possible leak on the memory tile, its value and the fact it comes from', async () => {
        const growth: RunGrowth = {
            startedAt: NOW - 7200,
            from: NOW - 5400,
            to: NOW,
            private: {
                perHour: 180 * 1024 ** 2,
                fit: 0.97,
                start: 9 * 1024 ** 3,
                points: 90,
                leak: true,
                residual: null,
                slopeError: null,
                center: 0,
            },
            handles: {
                perHour: 12,
                fit: 0.2,
                start: 3200,
                points: 90,
                leak: false,
                residual: null,
                slopeError: null,
                center: 0,
            },
        };
        const { say, live } = await page(hour(), { growth });
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: SAMPLE } });

        const memory = tile(en.tiles.memory.label);
        expect(memory.getByText(en.tiles.memory.possibleLeak)).not.toBeNull();
        expect(memory.getByText('9.4').className).toContain('text-warning');
        const since = memory.getByText('+180 MB/h');
        expect(since.className).toContain('text-warning');
        expect(memory.queryByText(en.tiles.memory.handles)).toBeNull();
        expect(card(en.tree.title).getByText('142 · 3,210 · +12/h')).not.toBeNull();
        live.stop();
    });

    it('should write a growth that rounds to nothing as 0, without a sign', async () => {
        const growth: RunGrowth = {
            startedAt: NOW - 7200,
            from: NOW - 5400,
            to: NOW,
            private: {
                perHour: 0.3,
                fit: 0.01,
                start: 9 * 1024 ** 3,
                points: 90,
                leak: false,
                residual: null,
                slopeError: null,
                center: 0,
            },
            handles: {
                perHour: -0.3,
                fit: 0.01,
                start: 3200,
                points: 90,
                leak: false,
                residual: null,
                slopeError: null,
                center: 0,
            },
        };
        const { say, live } = await page(hour(), { growth });
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: SAMPLE } });

        expect(tile(en.tiles.memory.label).getByText('0 bytes/h')).not.toBeNull();
        expect(card(en.tree.title).getByText('142 · 3,210 · 0/h')).not.toBeNull();
        live.stop();
    });

    it('should flag handles that keep growing with a fact of their own', async () => {
        const growth: RunGrowth = {
            startedAt: NOW - 7200,
            from: NOW - 5400,
            to: NOW,
            private: {
                perHour: 2 * 1024 ** 2,
                fit: 0.1,
                start: 9 * 1024 ** 3,
                points: 90,
                leak: false,
                residual: null,
                slopeError: null,
                center: 0,
            },
            handles: {
                perHour: 300,
                fit: 0.9,
                start: 3200,
                points: 90,
                leak: true,
                residual: null,
                slopeError: null,
                center: 0,
            },
        };
        const { say, live } = await page(hour(), { growth });
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: SAMPLE } });

        const memory = tile(en.tiles.memory.label);
        expect(memory.getByText(en.tiles.memory.possibleLeak)).not.toBeNull();
        expect(memory.getByText('9.4').className).not.toContain('text-warning');
        expect(memory.getByText('+2 MB/h').className).not.toContain('text-warning');
        expect(memory.getByText('+300/h since start').className).toContain('text-warning');
        live.stop();
    });

    it('should leave the CPU split empty and its keys unknown without the kernel time', async () => {
        const unsplit = hour();
        const { say, live } = await page({
            ...unsplit,
            series: unsplit.series.map((data) =>
                data.series === 'fxsKernel' ? { ...data, values: values(() => null) } : data,
            ),
        });
        const { fxsKernel: _, ...withoutKernel } = SAMPLE.values;
        say({
            type: 'snapshot',
            topic: 'metrics.live',
            data: { latest: { ...SAMPLE, values: withoutKernel } },
        });

        const cpu = tile(en.tiles.cpu.label);
        expect(cpu.getByText('user n/a')).not.toBeNull();
        expect(cpu.getByText('kernel n/a')).not.toBeNull();
        expect(document.querySelector('.bg-surface-3.rounded-pill')).not.toBeNull();
        live.stop();
    });

    it('should split the CPU over the hour, as one slot holds too few clock ticks to split', async () => {
        const ticks: MetricsSeries = {
            ...hour(),
            series: TILE_SERIES.map((series) => ({
                series,
                values: values((index) => {
                    if (series === 'fxsCpu') return index >= 710 ? 1 : null;
                    if (series === 'fxsKernel') return index >= 710 ? Number(index < 714) : null;
                    return null;
                }),
            })),
        };
        const { say, live } = await page(ticks);
        const allKernel = { ...SAMPLE.values, fxsCpu: 1, fxsKernel: 1 };
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: { ...SAMPLE, values: allKernel } } });

        const cpu = tile(en.tiles.cpu.label);
        expect(cpu.getByText('user 50%')).not.toBeNull();
        expect(cpu.getByText('kernel 50%')).not.toBeNull();
        live.stop();
    });

    it('should show steal and pressure where the host gives them, and a held clock as a warning', async () => {
        const { say, live } = await page();
        const linux = {
            ...SAMPLE.values,
            hostClockLimit: 72,
            hostSteal: 0.2,
            hostPressureCpu: 3,
            hostPressureMemory: 0,
            hostPressureIo: 1,
            hostDiskTransfers: 0,
            hostDiskFree: 118 * 1024 ** 3,
        };
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: { ...SAMPLE, values: linux } } });

        const host = tile(en.tiles.host.label);
        expect(host.getByText(en.tiles.host.limited('72%').trim()).className).toContain('text-warning');
        expect(host.getByText(en.tiles.host.steal).nextElementSibling?.textContent).toBe('0.2%');
        expect(host.getByText('cpu 3% · memory 0% · io 1%')).not.toBeNull();
        expect(host.getByText('118 GB · idle')).not.toBeNull();
        live.stop();
    });

    it('should give a fast disk its latency rather than read it as no time at all', async () => {
        const { say, live } = await page();
        const disk = {
            ...SAMPLE.values,
            hostDiskFree: 118 * 1024 ** 3,
            hostDiskTransfers: 640,
            hostDiskLatency: 0.022,
        };
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: { ...SAMPLE, values: disk } } });

        expect(tile(en.tiles.host.label).getByText('118 GB · 0.022 ms')).not.toBeNull();
        live.stop();
    });

    it('should give the tree card the rows each system measures', async () => {
        const windows = await page(hour(), { started: 4 });
        windows.say({
            type: 'snapshot',
            topic: 'metrics.live',
            data: { latest: { ...SAMPLE, values: { ...SAMPLE.values, treePrivatePeak: 1.5 * 1024 ** 3 } } },
        });
        const onWindows = card(en.tree.title);
        expect(onWindows.getByText(en.tree.startedLastHour).nextElementSibling?.textContent).toBe('4');
        expect(onWindows.getByText(en.tree.memoryPeak).nextElementSibling?.textContent).toBe('1.5 GB');
        windows.live.stop();
        cleanup();

        const linux = await page();
        linux.say({
            type: 'snapshot',
            topic: 'metrics.live',
            data: { latest: { ...SAMPLE, values: { ...SAMPLE.values, fxsHardFaults: 15 } } },
        });
        const onLinux = card(en.tree.title);
        expect(onLinux.queryByText(en.tree.startedLastHour)).toBeNull();
        expect(onLinux.queryByText(en.tree.memoryPeak)).toBeNull();
        expect(onLinux.getByText('240/s · 3 hard/s')).not.toBeNull();
        linux.live.stop();
    });

    it('should not read the few hard faults of a slot as none', async () => {
        const { say, live } = await page();
        const few = { ...SAMPLE.values, fxsFaults: 40, fxsHardFaults: 2 };
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: { ...SAMPLE, values: few } } });

        expect(card(en.tree.title).getByText(en.tree.faults).nextElementSibling?.textContent).toBe(
            '8/s · 0.4 hard/s',
        );
        live.stop();
    });

    it('should list the busiest threads, an unnamed one, then the rest of the process as Others', async () => {
        const { live } = await page(hour(), {
            threads: {
                from: NOW - 3595,
                to: NOW + 5,
                measured: 720,
                process: 90,
                threads: [
                    { name: 'UV loop: svMain', group: 'main', cpu: 58 },
                    { name: '', group: 'other', cpu: 12 },
                    { name: 'UV loop: svNetw', group: 'network', cpu: 9 },
                ],
            },
        });

        const rows = [...card(en.threads.title).getAllByRole('row')].slice(1);
        expect(rows.map((row) => row.textContent)).toEqual([
            'UV loop: svMain64.4%58%',
            `${en.threads.unnamed}13.3%12%`,
            'UV loop: svNetw10%9%',
            `${en.threads.others}12.2%11%`,
        ]);
        live.stop();
    });

    it('should keep the unnamed threads and Others apart when the hour is read again', async () => {
        const { client, live } = await page(hour(), {
            threads: shares(10, [
                ['UV loop: svMain', 6],
                ['', 2],
            ]),
        });

        await act(async () => {
            client.setQueryData(
                threadsQuery.queryKey,
                shares(9, [
                    ['', 4],
                    ['UV loop: svMain', 3],
                ]),
            );
            await new Promise((resolve) => setTimeout(resolve, 0));
        });

        const rows = [...card(en.threads.title).getAllByRole('row')].slice(1);
        expect(rows.map((row) => row.textContent)).toEqual([
            `${en.threads.unnamed}44.4%4%`,
            'UV loop: svMain33.3%3%',
            `${en.threads.others}22.2%2%`,
        ]);
        live.stop();
    });

    it('should say no thread was measured over an hour without any', async () => {
        const { live } = await page();

        expect(card(en.threads.title).getByText(en.threads.empty)).not.toBeNull();
        live.stop();
    });

    it('should leave the share of the machine unknown while the host has not said its processors', async () => {
        const { say, live } = await page();
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: { ...SAMPLE, cores: null } } });

        const cpu = tile(en.tiles.cpu.label);
        expect(cpu.getByText('182%')).not.toBeNull();
        expect(cpu.getByText(en.tiles.cpu.machine).nextElementSibling?.textContent).toBe(en.tiles.unknown);
        live.stop();
    });

    it('should say there was no hitch only over an hour in which svMain was followed', async () => {
        const followed = await page(quietHour(['hitchesMain', 'hitchMax']));
        expect(lastHitch()).toBe(en.tiles.tick.noHitch);
        followed.live.stop();
        cleanup();

        const unmeasured = await page(quietHour([]));
        expect(lastHitch()).toBe(en.tiles.unknown);
        unmeasured.live.stop();
    });

    it('should show n/a rather than zero before any sample', async () => {
        const { live } = await page();

        const players = tile(en.tiles.players.label);
        expect(players.getAllByText('n/a').length).toBeGreaterThanOrEqual(2);
        expect(players.queryByText('0')).toBeNull();
        for (const label of [
            en.tiles.players.label,
            en.tiles.tick.label,
            en.tiles.cpu.label,
            en.tiles.memory.label,
            en.tiles.host.label,
            en.tiles.observer.label,
        ]) {
            expect(figure(label), label).toBe(en.tiles.unknown);
        }
        live.stop();
    });

    it('should stop showing the last sample as current once the link to Observer is gone', async () => {
        const { say, drop, live } = await page();
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: SAMPLE } });
        expect(tile(en.tiles.players.label).queryByText('187')).not.toBeNull();

        drop();

        expect(tile(en.tiles.players.label).queryByText('187')).toBeNull();
        expect(tile(en.tiles.host.label).queryByText('41%')).toBeNull();
        expect(tile(en.tiles.host.label).getByRole('img', { name: en.tiles.host.trend })).not.toBeNull();
        live.stop();
    });

    it('should show no sample from before a drop as current when Observer comes back without one yet', async () => {
        const { say, drop, live } = await page();
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: SAMPLE } });
        drop();

        // The first retry comes a second after the drop, to an Observer that has just restarted.
        await act(() => new Promise((resolve) => setTimeout(resolve, 1200)));
        say({ type: 'hello', connectionId: 2, contract: 1 });
        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: null } });

        expect(figure(en.tiles.players.label)).toBe(en.tiles.unknown);
        expect(figure(en.tiles.host.label)).toBe(en.tiles.unknown);

        say({
            type: 'event',
            topic: 'metrics.live',
            data: { ...SAMPLE, ts: SAMPLE.ts + 5, values: { ...SAMPLE.values, players: 12 } },
        });

        expect(figure(en.tiles.players.label)).toBe('12/ 256');
        live.stop();
    });

    it('should stop showing when Observer started once the link to it is gone', async () => {
        const { drop, live } = await page();
        expect(tile(en.tiles.observer.label).queryByText(/^0\.1\.0 · up \d min$/)).not.toBeNull();

        drop();

        expect(tile(en.tiles.observer.label).queryByText(/^0\.1\.0 · up \d min$/)).toBeNull();
        live.stop();
    });

    it('should tell the owner the hang delay that ran out, and the way to the card that sets it', async () => {
        const { live, getByText, getByRole } = await hungFor(OWNER);

        getByText(en.server_state.hangNoHttp);
        getByText(en.server_state.hangAdvice(15));
        expect(getByRole('link', { name: en.server_state.openHang }).getAttribute('href')).toBe(
            '/server#server-hang',
        );
        live.stop();
        vi.unstubAllGlobals();
    });

    it('should leave the hang delay out for another account, which cannot change it', async () => {
        const { live, getByText, queryByText, queryByRole } = await hungFor({
            ...OWNER,
            id: 2,
            username: 'grace',
            isOwner: false,
        });

        getByText(en.server_state.hangNoHttp);
        expect(queryByText(en.server_state.hangAdvice(15))).toBeNull();
        expect(queryByRole('link', { name: en.server_state.openHang })).toBeNull();
        live.stop();
        vi.unstubAllGlobals();
    });

    it('should say Observer cannot be reached rather than that it runs once it stops answering', async () => {
        const { client, live } = await page();

        await act(async () => {
            const query = client.getQueryCache().find({ queryKey: healthQuery.queryKey });
            query?.setState({ status: 'error', error: new ApiError(502, 'internal'), fetchStatus: 'idle' });
            await new Promise((resolve) => setTimeout(resolve, 0));
        });

        const observer = tile(en.tiles.observer.label);
        expect(observer.queryByText(en.tiles.observer.unreachable)).not.toBeNull();
        expect(observer.queryByText(en.tiles.observer.running)).toBeNull();
        live.stop();
    });
});
