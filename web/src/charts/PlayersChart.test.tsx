import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, render } from '@testing-library/react';
import { AreaSeries, LineSeries } from 'lightweight-charts';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { LiveSample } from '../api/generated/LiveSample';
import type { MetricsSeries } from '../api/generated/MetricsSeries';
import { seriesQuery } from '../api/metrics';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { LiveMetricsProvider } from '../metrics/live';
import PlayersChart from './PlayersChart';

// The library draws on a canvas, which the test DOM does not have. This one keeps what each series
// was given to draw, and the scale it was given.
const library = vi.hoisted(() => {
    type Drawn = { kind: 'set' | 'update'; points: Record<string, unknown>[] };
    type Series = { type: unknown; options: Record<string, unknown>; drawn: Drawn[] };
    const series: Series[] = [];
    const createChart = () => ({
        addSeries: (type: unknown, options: Record<string, unknown>) => {
            const added: Series = { type, options, drawn: [] };
            series.push(added);
            return {
                setData: (points: Record<string, unknown>[]) => added.drawn.push({ kind: 'set', points }),
                update: (point: Record<string, unknown>) =>
                    added.drawn.push({ kind: 'update', points: [point] }),
                applyOptions: () => {},
                priceToCoordinate: () => null,
                attachPrimitive: () => {},
            };
        },
        applyOptions: () => {},
        priceScale: () => ({ width: () => 38 }),
        timeScale: () => ({
            fitContent: () => {},
            timeToCoordinate: () => null,
            subscribeVisibleLogicalRangeChange: () => {},
            unsubscribeVisibleLogicalRangeChange: () => {},
        }),
        subscribeCrosshairMove: () => {},
        unsubscribeCrosshairMove: () => {},
        remove: () => {},
    });
    return { series, createChart };
});

vi.mock('lightweight-charts', async (importOriginal) => ({
    ...(await importOriginal<typeof import('lightweight-charts')>()),
    createChart: library.createChart,
}));

/** Every colour token reads as the accent, the way the chart resolves them through a canvas. */
class TestCanvas {
    getContext() {
        return {
            fillStyle: '',
            clearRect: () => {},
            fillRect: () => {},
            getImageData: () => ({ data: [37, 99, 235, 255] }),
        };
    }
}

const TRANSPARENT = 'rgba(37, 99, 235, 0)';

class TestResizeObserver {
    observe() {}
    disconnect() {}
}

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const NOW = Math.floor(Date.now() / 5000) * 5;
/** The end of the day's grid: its last two-minute bucket, still open, holds now. */
const DAY_END = (Math.floor(NOW / 120) + 1) * 120;

/** A gauge over the day's 720 buckets of two minutes, with its range and its count of samples. */
const gauge = (series: 'players' | 'slots', at: (index: number) => number | null) => {
    const values = Array.from({ length: 720 }, (_, index) => at(index));
    return {
        series,
        values,
        min: values,
        max: values,
        samples: values.map((value) => (value === null ? 0 : 24)),
    };
};

/** A day as Observer reads it. */
const day = (
    players: (index: number) => number | null,
    slots: (index: number) => number | null,
): MetricsSeries => ({
    range: '24h',
    step: 120,
    from: DAY_END - 86_400,
    buckets: 720,
    last: NOW - 5,
    series: [gauge('players', players), gauge('slots', slots)],
    markers: [],
    observed: Array.from({ length: 720 }, (_, index) => (players(index) === null ? 0 : 24)),
    online: Array.from({ length: 720 }, (_, index) => (players(index) === null ? 0 : 24)),
});

const sample = (players: number, slots: number): LiveSample => ({
    ts: NOW,
    state: 'online',
    values: { players, slots },
    children: [],
    cores: 16,
});

/** The chart over `read`, the day as the page last read it, and a socket the test speaks for. */
const chart = async (read: MetricsSeries) => {
    const client = new QueryClient({
        defaultOptions: { queries: { staleTime: Infinity, retry: false, refetchInterval: false } },
    });
    client.setQueryData(seriesQuery('24h', ['players', 'slots']).queryKey, read);
    let socket: SocketEvents | undefined;
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: () => {}, close: () => {} };
        },
        refetch: () => {},
    });
    render(
        <QueryClientProvider client={client}>
            <I18nProvider>
                <LiveContext value={live}>
                    <LiveMetricsProvider>
                        <PlayersChart />
                    </LiveMetricsProvider>
                </LiveContext>
            </I18nProvider>
        </QueryClientProvider>,
    );
    live.start();
    await settle();
    const say = (message: unknown) => act(() => socket?.message(JSON.stringify(message)));
    say({ type: 'hello', connectionId: 1, contract: 1 });
    return { live, say };
};

const drawn = (type: unknown) => {
    const series = library.series.find((added) => added.type === type);
    if (series === undefined) {
        throw new Error('the series was not added');
    }
    return series;
};

const scaleTop = (type: unknown) => {
    const autoscale = drawn(type).options.autoscaleInfoProvider as () => { priceRange: { maxValue: number } };
    return autoscale().priceRange.maxValue;
};

describe('PlayersChart', () => {
    beforeEach(() => {
        vi.stubGlobal('OffscreenCanvas', TestCanvas);
        vi.stubGlobal('ResizeObserver', TestResizeObserver);
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        library.series.length = 0;
    });

    it('should draw the whole line again when a live sample picks it up after a stop', async () => {
        // The server stopped 40 minutes ago, twenty buckets before the open one.
        const { live, say } = await chart(
            day(
                (index) => (index < 700 ? 37 : null),
                () => 64,
            ),
        );

        say({ type: 'snapshot', topic: 'metrics.live', data: { latest: sample(41, 64) } });

        const players = drawn(AreaSeries).drawn.at(-1);
        expect(players?.kind).toBe('set');
        expect(players?.points[699]).toMatchObject({
            value: 37,
            lineColor: TRANSPARENT,
            topColor: TRANSPARENT,
        });
        expect(players?.points[700]).not.toHaveProperty('value');
        expect(players?.points[719]).toEqual({ time: DAY_END - 120, value: 41 });
        live.stop();
    });

    it('should fit the scale to the most slots of the range, not to the latest', async () => {
        // The limit was 512 slots this morning, and is 64 since a restart.
        const { live } = await chart(
            day(
                () => 37,
                (index) => (index < 300 ? 512 : 64),
            ),
        );

        expect(scaleTop(AreaSeries)).toBe(1000);
        expect(scaleTop(LineSeries)).toBe(1000);
        live.stop();
    });
});
