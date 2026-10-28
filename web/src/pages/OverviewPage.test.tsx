import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { ApiError } from '../api/client';
import { healthQuery } from '../api/health';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { OverviewPage } from './OverviewPage';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** The page with Observer's health already read and a socket the test speaks for. */
const page = async () => {
    const client = new QueryClient({
        defaultOptions: { queries: { staleTime: Infinity, retry: false, refetchInterval: false } },
    });
    client.setQueryData(healthQuery.queryKey, { status: 'ok', version: '0.1.0' });
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
    act(() => {
        socket?.message(JSON.stringify({ type: 'hello', connectionId: 1, contract: 1 }));
        socket?.message(
            JSON.stringify({
                type: 'snapshot',
                topic: 'status',
                data: { version: '0.1.0', startedAt: new Date(Date.now() - 300_000).toISOString() },
            }),
        );
    });
    return { ...view, client, live, drop: () => act(() => socket?.close(1006)) };
};

describe('OverviewPage', () => {
    afterEach(cleanup);

    it('should stop showing when Observer started once the link to it is gone', async () => {
        const { queryByText, drop, live } = await page();
        expect(queryByText(/^Started /)).not.toBeNull();

        drop();

        expect(queryByText(/^Started /)).toBeNull();
        live.stop();
    });

    it('should say Observer cannot be reached rather than that it runs once it stops answering', async () => {
        const { queryByText, client, live } = await page();

        await act(async () => {
            const query = client.getQueryCache().find({ queryKey: healthQuery.queryKey });
            query?.setState({ status: 'error', error: new ApiError(502, 'internal'), fetchStatus: 'idle' });
            await new Promise((resolve) => setTimeout(resolve, 0));
        });

        expect(queryByText(en.overview.unreachable)).not.toBeNull();
        expect(queryByText(en.overview.running)).toBeNull();
        live.stop();
    });
});
