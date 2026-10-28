import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { Realtime, type SocketEvents } from './realtime';

/** A socket Observer would have opened, driven by hand. */
class FakeSocket {
    readonly sent: unknown[] = [];
    closed = false;

    constructor(
        readonly ticket: string,
        private readonly events: SocketEvents,
    ) {}

    send(text: string) {
        this.sent.push(JSON.parse(text));
    }

    close() {
        this.closed = true;
    }

    receive(message: unknown) {
        this.events.message(typeof message === 'string' ? message : JSON.stringify(message));
    }

    hello(contract = 1) {
        this.receive({ type: 'hello', connectionId: 1, contract });
    }

    drop(code = 1006) {
        this.events.close(code);
    }
}

const STATUS = { version: '0.1.0', startedAt: '2026-10-28T09:00:00Z' };

const auditEntry = (id: number) => ({
    id,
    at: '2026-10-28T09:00:00.120Z',
    actorKind: 'user',
    actorId: 1,
    actorName: 'ada',
    ip: '203.0.113.7',
    userAgent: 'Firefox',
    action: 'reauthenticated',
    target: 'ada',
    outcome: 'success',
    details: {},
});

const setup = () => {
    const sockets: FakeSocket[] = [];
    const refetch = vi.fn();
    const ticket = vi.fn(async (): Promise<string | null> => `ticket-${sockets.length + 1}`);
    const live = new Realtime({
        ticket,
        connect: (value, events) => {
            const socket = new FakeSocket(value, events);
            sockets.push(socket);
            return socket;
        },
        refetch,
    });
    const last = () => {
        const socket = sockets.at(-1);
        if (socket === undefined) {
            throw new Error('no socket was opened');
        }
        return socket;
    };
    return { live, sockets, last, refetch, ticket };
};

/** Lets the ticket request and whatever timers are due run. */
const after = (milliseconds: number) => vi.advanceTimersByTimeAsync(milliseconds);

const setVisibility = (state: DocumentVisibilityState) => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, value: state });
    document.dispatchEvent(new Event('visibilitychange'));
};

describe('Realtime', () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
        setVisibility('visible');
    });

    it('should be live once Observer says hello, not when the socket opens', async () => {
        const { live, last } = setup();

        live.start();
        expect(live.getState()).toBe('connecting');
        await after(0);
        expect(last().ticket).toBe('ticket-1');
        expect(live.getState()).toBe('connecting');

        last().hello();
        expect(live.getState()).toBe('live');
    });

    it('should subscribe to a watched topic and hand it snapshots and events', async () => {
        const { live, last } = setup();
        const snapshot = vi.fn();
        const event = vi.fn();
        live.watch('audit', { snapshot, event });
        live.start();
        await after(0);
        last().hello();

        last().receive({ type: 'snapshot', topic: 'audit', data: { latestId: 41 } });
        last().receive({ type: 'event', topic: 'audit', data: auditEntry(42) });

        expect(last().sent).toEqual([{ type: 'sub', topic: 'audit' }]);
        expect(snapshot).toHaveBeenCalledWith({ latestId: 41 });
        expect(event).toHaveBeenCalledWith(auditEntry(42));
    });

    it('should share a topic between watchers and leave it with the last one', async () => {
        const { live, last } = setup();
        live.start();
        await after(0);
        last().hello();
        const first = vi.fn();
        const second = vi.fn();

        const stopFirst = live.watch('status', { snapshot: first });
        last().receive({ type: 'snapshot', topic: 'status', data: STATUS });
        const stopSecond = live.watch('status', { snapshot: second });
        stopFirst();
        stopSecond();

        expect(second).toHaveBeenCalledWith(STATUS);
        expect(last().sent).toEqual([
            { type: 'sub', topic: 'status' },
            { type: 'unsub', topic: 'status' },
        ]);
    });

    it('should ignore every message the contract does not describe', async () => {
        const { live, last } = setup();
        const snapshot = vi.fn();
        const event = vi.fn();
        live.watch('audit', { snapshot, event });
        live.start();
        await after(0);
        last().hello();

        last().receive('not json');
        last().receive({ type: 'snapshot', topic: 'audit', data: { latestId: '41' } });
        last().receive({ type: 'snapshot', topic: 'console', data: {} });
        last().receive({ type: 'event', topic: 'audit', data: { ...auditEntry(42), action: 'rm_rf' } });
        last().receive({ type: 'event', topic: 'audit', data: { ...auditEntry(42), id: 1.5 } });
        last().receive({ type: 'event', topic: 'status', data: STATUS });

        expect(snapshot).not.toHaveBeenCalled();
        expect(event).not.toHaveBeenCalled();
    });

    it('should resubscribe every topic and read the queries again after a reconnect', async () => {
        const { live, last, sockets, refetch } = setup();
        live.watch('status', {});
        live.watch('audit', {});
        live.start();
        await after(0);
        last().hello();
        expect(refetch).not.toHaveBeenCalled();

        last().drop();
        expect(live.getState()).toBe('connecting');
        await after(1_000);
        last().hello();

        expect(sockets).toHaveLength(2);
        expect(live.getState()).toBe('live');
        expect(last().sent).toEqual([
            { type: 'sub', topic: 'status' },
            { type: 'sub', topic: 'audit' },
        ]);
        expect(refetch).toHaveBeenCalledOnce();
    });

    it('should read the queries again once a socket opens after attempts that failed', async () => {
        const { live, last, ticket, refetch } = setup();
        ticket.mockRejectedValueOnce(new TypeError('Failed to fetch'));
        live.start();
        await after(0);
        expect(live.getState()).toBe('connecting');

        await after(1_000);
        last().hello();

        expect(live.getState()).toBe('live');
        expect(refetch).toHaveBeenCalledOnce();
    });

    it('should tell a watcher its snapshot no longer holds once the link drops', async () => {
        const { live, last } = setup();
        const cleared = vi.fn();
        live.watch('status', { cleared });
        live.start();
        await after(0);
        last().hello();
        last().receive({ type: 'snapshot', topic: 'status', data: STATUS });
        expect(cleared).not.toHaveBeenCalled();

        last().drop();

        expect(cleared).toHaveBeenCalledOnce();
    });

    it('should start a topic over when Observer says it fell behind', async () => {
        const { live, last } = setup();
        live.watch('audit', {});
        live.start();
        await after(0);
        last().hello();

        last().receive({ type: 'resync', topic: 'audit' });

        expect(last().sent).toEqual([
            { type: 'sub', topic: 'audit' },
            { type: 'sub', topic: 'audit' },
        ]);
    });

    it('should back off, then poll every minute once down for thirty seconds', async () => {
        const { live, last, ticket, refetch } = setup();
        live.start();
        await after(0);
        last().hello();
        ticket.mockRejectedValue(new TypeError('Failed to fetch'));

        last().drop();
        await after(29_000);
        expect(live.getState()).toBe('connecting');
        // Retries 1, 2, 4 and 8 seconds apart: four within the first 29 seconds.
        expect(ticket).toHaveBeenCalledTimes(1 + 4);

        await after(1_000);
        expect(live.getState()).toBe('offline');
        expect(refetch).not.toHaveBeenCalled();
        await after(60_000);
        expect(refetch).toHaveBeenCalledOnce();
        setVisibility('hidden');
        await after(60_000);
        expect(refetch).toHaveBeenCalledOnce();
    });

    it('should stop, and let the queries find out, once the session ended', async () => {
        const { live, last, sockets, refetch } = setup();
        live.start();
        await after(0);
        last().hello();

        last().drop(4401);
        await after(120_000);

        expect(live.getState()).toBe('refused');
        expect(refetch).toHaveBeenCalledOnce();
        expect(sockets).toHaveLength(1);
    });

    it('should be refused without a socket when no ticket is given', async () => {
        const { live, sockets, ticket } = setup();
        ticket.mockResolvedValue(null);

        live.start();
        await after(60_000);

        expect(live.getState()).toBe('refused');
        expect(sockets).toHaveLength(0);
    });

    it('should be refused by an Observer speaking another contract', async () => {
        const { live, last } = setup();
        live.start();
        await after(0);

        last().hello(2);

        expect(live.getState()).toBe('refused');
        expect(last().closed).toBe(true);
    });

    it('should tell a watcher its topic was refused', async () => {
        const { live, last } = setup();
        const refused = vi.fn();
        live.watch('audit', { refused });
        live.start();
        await after(0);
        last().hello();

        last().receive({ type: 'unsubscribed', topic: 'audit', reason: 'forbidden' });

        expect(refused).toHaveBeenCalledOnce();
    });

    it('should ping, and give up on a socket silent for a minute', async () => {
        const { live, last, sockets } = setup();
        live.start();
        await after(0);
        last().hello();

        await after(20_000);
        expect(last().sent).toEqual([{ type: 'ping' }]);
        last().receive({ type: 'pong' });
        await after(40_000);
        expect(sockets[0]?.closed).toBe(false);
        await after(21_000);

        expect(sockets[0]?.closed).toBe(true);
        expect(live.getState()).toBe('connecting');
        expect(sockets).toHaveLength(2);
    });

    it('should read the queries again when a tab hidden for five minutes comes back', async () => {
        const { live, last, refetch } = setup();
        live.start();
        await after(0);
        last().hello();

        setVisibility('hidden');
        await after(4 * 60_000);
        setVisibility('visible');
        expect(refetch).not.toHaveBeenCalled();
        setVisibility('hidden');
        await after(5 * 60_000);
        setVisibility('visible');

        expect(refetch).toHaveBeenCalledOnce();
    });

    it('should close the socket and stop retrying once stopped', async () => {
        const { live, last, sockets } = setup();
        live.start();
        await after(0);
        last().hello();

        live.stop();
        last().drop();
        await after(120_000);

        expect(last().closed).toBe(true);
        expect(live.getState()).toBe('offline');
        expect(sockets).toHaveLength(1);
    });
});
