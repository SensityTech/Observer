import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { MetricsSeries } from '../api/generated/MetricsSeries';
import { seriesQuery } from '../api/metrics';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime } from '../live/realtime';
import { LiveMetricsProvider } from '../metrics/live';
import { clock } from './markers';
import { TICK_SERIES } from './tick';
import TickChart from './TickChart';

// The library draws on a canvas, which the test DOM does not have.
vi.mock('lightweight-charts', async (importOriginal) => ({
    ...(await importOriginal<typeof import('lightweight-charts')>()),
    createChart: () => ({
        addSeries: () => ({
            setData: () => {},
            update: () => {},
            applyOptions: () => {},
            priceToCoordinate: () => null,
            attachPrimitive: () => {},
            createPriceLine: () => ({ applyOptions: () => {} }),
        }),
        applyOptions: () => {},
        priceScale: () => ({ width: () => 38, applyOptions: () => {} }),
        timeScale: () => ({
            fitContent: () => {},
            timeToCoordinate: () => null,
            subscribeVisibleLogicalRangeChange: () => {},
            unsubscribeVisibleLogicalRangeChange: () => {},
        }),
        subscribeCrosshairMove: () => {},
        unsubscribeCrosshairMove: () => {},
        remove: () => {},
    }),
}));

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

class TestResizeObserver {
    observe() {}
    disconnect() {}
}

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const NOW = Math.floor(Date.now() / 30_000) * 30;
const FROM = NOW - 21_570;

const sampled = (index: number) => index >= 600;
const down = (index: number) => index >= 694 && index < 700;

/** Six hours of 30 s buckets, ticks measured from the 600th, the server down for the 6 before the 700th. */
const sixHours = (): MetricsSeries => {
    return {
        range: '6h',
        step: 30,
        from: FROM,
        buckets: 720,
        last: NOW - 5,
        series: TICK_SERIES.map((series) => ({
            series,
            values: Array.from({ length: 720 }, (_, index) =>
                sampled(index) && !down(index) && series !== 'hitchMax' ? 4 : null,
            ),
        })),
        markers: [{ at: FROM + 700 * 30, kind: 'start', backOnlineAt: null }],
        observed: Array.from({ length: 720 }, (_, index) => (sampled(index) ? 6 : 0)),
        online: Array.from({ length: 720 }, (_, index) => (sampled(index) && !down(index) ? 6 : 0)),
    };
};

const chart = async (read: MetricsSeries) => {
    const client = new QueryClient({
        defaultOptions: { queries: { staleTime: Infinity, retry: false, refetchInterval: false } },
    });
    client.setQueryData(seriesQuery('6h', TICK_SERIES).queryKey, read);
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: () => ({ send: () => {}, close: () => {} }),
        refetch: () => {},
    });
    render(
        <QueryClientProvider client={client}>
            <I18nProvider>
                <LiveContext value={live}>
                    <LiveMetricsProvider>
                        <TickChart />
                    </LiveMetricsProvider>
                </LiveContext>
            </I18nProvider>
        </QueryClientProvider>,
    );
    await settle();
    return live;
};

describe('TickChart', () => {
    beforeEach(() => {
        vi.stubGlobal('OffscreenCanvas', TestCanvas);
        vi.stubGlobal('ResizeObserver', TestResizeObserver);
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should say how long the server was down along with the markers, the plot being hidden', async () => {
        const live = await chart(sixHours());

        expect(screen.getByText(`Down for 3 min over 6h. Start ${clock(FROM + 700 * 30)}`)).not.toBeNull();
        live.stop();
    });

    it('should name its range control after the chart', async () => {
        const live = await chart(sixHours());

        expect(screen.getByRole('group', { name: en.charts.tick.range })).not.toBeNull();
        live.stop();
    });
});
