import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, render, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { ApiError } from '../api/client';
import type { LiveSample } from '../api/generated/LiveSample';
import type { MetricsSeries } from '../api/generated/MetricsSeries';
import { healthQuery } from '../api/health';
import { seriesQuery } from '../api/metrics';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { TILE_SERIES } from '../overview/Tiles';
import { OverviewPage } from './OverviewPage';

const chart = vi.hoisted(() => ({ fails: false }));

// The chart draws on a canvas, which the test DOM does not have.
vi.mock('../charts/PlayersChart', () => ({
    default: () => {
        if (chart.fails) {
            throw new Error('the chart could not be drawn');
        }
        return null;
    },
}));

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const NOW = Math.floor(Date.now() / 5000) * 5;

const values = (at: (index: number) => number | null) => Array.from({ length: 720 }, (_, index) => at(index));

/** An hour with players in its last slots and a hitch ten minutes ago. */
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
                return index > 700 ? 10 + (index % 3) : null;
            }),
        })),
        markers: [],
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

/** The page with Observer's health and the last hour already read, and a socket the test speaks for. */
const page = async (lastHour: MetricsSeries = hour()) => {
    const client = new QueryClient({
        defaultOptions: { queries: { staleTime: Infinity, retry: false, refetchInterval: false } },
    });
    client.setQueryData(healthQuery.queryKey, { status: 'ok', version: '0.1.0' });
    client.setQueryData(seriesQuery('1h', TILE_SERIES).queryKey, lastHour);
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
        <QueryClientProvider client={client}>
            <I18nProvider>
                <LiveContext value={live}>
                    <OverviewPage />
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
        expect(cpu.getByText('142 · 3,210')).not.toBeNull();
        expect(cpu.getByText('3 processes · 4.1%')).not.toBeNull();
        const memory = tile(en.tiles.memory.label);
        expect(memory.getByText('9.4').textContent).toBe('9.4GB private');
        expect(memory.getByText('10.1 GB')).not.toBeNull();
        expect(memory.getAllByText('n/a')).toHaveLength(2);
        const host = tile(en.tiles.host.label);
        expect(host.getByText('22.1 GB / 64 GB')).not.toBeNull();
        expect(host.getByText('12 Mbit/s · 2 Mbit/s')).not.toBeNull();
        const observer = tile(en.tiles.observer.label);
        expect(observer.getByText('38').textContent).toBe('38MB RAM');
        expect(observer.getByText(en.tiles.observer.running)).not.toBeNull();
        expect(observer.getByText(/^0\.1\.0 · up \d min$/)).not.toBeNull();
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

    it('should stop showing when Observer started once the link to it is gone', async () => {
        const { drop, live } = await page();
        expect(tile(en.tiles.observer.label).queryByText(/^0\.1\.0 · up \d min$/)).not.toBeNull();

        drop();

        expect(tile(en.tiles.observer.label).queryByText(/^0\.1\.0 · up \d min$/)).toBeNull();
        live.stop();
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
