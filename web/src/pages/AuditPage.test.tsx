import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { auditQuery } from '../api/audit';
import type { AuditEntry } from '../api/generated/AuditEntry';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { AuditPage } from './AuditPage';

const entry = (id: number): AuditEntry => ({
    id,
    at: '2026-10-28T10:00:00.000Z',
    actorKind: 'cli',
    actorId: null,
    actorName: null,
    ip: null,
    userAgent: null,
    action: 'user_unlocked',
    target: `user-${id}`,
    outcome: 'success',
    details: {},
});

/** Lets the query and the socket say what they have to say. */
const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** The page over a log of entries 1 to `newest` already read, with a socket the test speaks for. */
const page = async (newest: number) => {
    const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity, retry: false } } });
    const entries = Array.from({ length: newest }, (_, index) => entry(newest - index));
    client.setQueryData(auditQuery({}).queryKey, {
        pages: [{ entries, next: null }],
        pageParams: [undefined],
    });
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
                    <AuditPage />
                </LiveContext>
            </I18nProvider>
        </QueryClientProvider>,
    );
    live.start();
    await settle();
    const say = async (message: unknown) => {
        act(() => socket?.message(JSON.stringify(message)));
        await settle();
    };
    await say({ type: 'hello', connectionId: 1, contract: 1 });
    return { ...view, live, say };
};

describe('AuditPage', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should keep the entries that came live when the same filter is applied again', async () => {
        vi.stubGlobal(
            'fetch',
            vi.fn(async () => Response.json({ entries: [entry(4), entry(3)], next: null })),
        );
        const { say, getByText, getAllByRole, live } = await page(4);
        await say({ type: 'snapshot', topic: 'audit', data: { latestId: 4 } });
        await say({ type: 'event', topic: 'audit', data: entry(5) });
        expect(getAllByRole('row')).toHaveLength(1 + 5);

        fireEvent.click(getByText(en.audit.apply));
        await settle();
        expect(getAllByRole('row')).toHaveLength(1 + 5);

        fireEvent.click(getByText(en.audit.clear));
        await settle();
        expect(getAllByRole('row')).toHaveLength(1 + 5);
        live.stop();
    });
});
