import type { ClientMessage } from '../api/generated/ClientMessage';
import type { Topic } from '../api/generated/Topic';
import type { TopicEvent } from '../api/generated/TopicEvent';
import type { TopicSnapshot } from '../api/generated/TopicSnapshot';
import { parseServerMessage } from './messages';

/** The subprotocol and contract this panel speaks; Observer's `hello` says which it speaks. */
export const PROTOCOL = 'observer.v1';
const CONTRACT = 1;
/** Observer closes a socket with this code once its session has ended. */
const CLOSE_SESSION_ENDED = 4401;

const PING_EVERY = 20_000;
/** A socket that brings nothing for this long, not even a pong, is gone. */
const SILENCE_LIMIT = 60_000;
const RETRY_DELAYS = [1_000, 2_000, 4_000, 8_000, 15_000, 30_000];
/** Down for this long, the link is offline and the panel polls instead. */
const OFFLINE_AFTER = 30_000;
const POLL_EVERY = 60_000;
/** A tab shown again after this long hidden reads everything again. */
const STALE_AFTER = 5 * 60_000;

export type LinkState = 'connecting' | 'live' | 'offline' | 'refused';

type Snapshots = { [Snapshot in TopicSnapshot as Snapshot['topic']]: Snapshot['data'] };
type Events = { [Event in TopicEvent as Event['topic']]: Event['data'] };

export type SnapshotOf<T extends Topic> = Snapshots[T];
/** Never for a topic that only has a snapshot. */
export type EventOf<T extends Topic> = T extends keyof Events ? Events[T] : never;

export type TopicHandlers<T extends Topic> = {
    snapshot?: (data: SnapshotOf<T>) => void;
    event?: (data: EventOf<T>) => void;
    /** The session does not hold the topic's permission. */
    refused?: () => void;
};

type Handlers = {
    snapshot?: (data: unknown) => void;
    event?: (data: unknown) => void;
    refused?: () => void;
};

/** One socket as the module drives it. */
export type Socket = { send: (text: string) => void; close: () => void };
export type SocketEvents = { message: (text: string) => void; close: (code: number) => void };

export type RealtimeOptions = {
    /** A one-time ticket for the next socket, or null once the session gets none. */
    ticket: () => Promise<string | null>;
    connect: (ticket: string, events: SocketEvents) => Socket;
    /** Reads the panel's queries again: after a reconnect, while offline, for a stale tab. */
    refetch: () => void;
};

type Timer = ReturnType<typeof setTimeout>;

/**
 * The panel's websocket, and the topics followed on it. It lives outside React: components read its
 * state with `useSyncExternalStore` and follow topics with `useTopic`, so no render ever opens a
 * second socket or drops a subscription. What it shows can always be read again over HTTP: a lost
 * message costs freshness, never correctness.
 */
export class Realtime {
    readonly #options: RealtimeOptions;
    readonly #listeners = new Set<() => void>();
    readonly #watchers = new Map<Topic, Set<Handlers>>();
    /** The last snapshot of each topic followed, for a component that starts watching later. */
    readonly #snapshots = new Map<Topic, unknown>();
    #state: LinkState = 'offline';
    #running = false;
    #socket: Socket | null = null;
    #live = false;
    /** Bumped for each socket, so that whatever a socket given up on still says is ignored. */
    #generation = 0;
    #failures = 0;
    /** Set once a socket said hello: the next hello is a reconnect, after which pages read again. */
    #wasLive = false;
    #heardAt = 0;
    #hiddenAt: number | null = null;
    #retry: Timer | undefined;
    #offline: Timer | undefined;
    #poll: Timer | undefined;
    #heartbeat: Timer | undefined;

    constructor(options: RealtimeOptions) {
        this.#options = options;
    }

    getState = (): LinkState => this.#state;

    subscribe = (listener: () => void) => {
        this.#listeners.add(listener);
        return () => {
            this.#listeners.delete(listener);
        };
    };

    start() {
        if (this.#running) {
            return;
        }
        this.#running = true;
        this.#wasLive = false;
        document.addEventListener('visibilitychange', this.#visibilityChanged);
        this.#setState('connecting');
        void this.#connect();
    }

    stop() {
        if (!this.#running) {
            return;
        }
        this.#running = false;
        document.removeEventListener('visibilitychange', this.#visibilityChanged);
        this.#dropSocket();
        this.#clearTimers();
        this.#setState('offline');
    }

    /** Follows a topic until the returned function is called. Topics are shared between watchers. */
    watch<T extends Topic>(topic: T, handlers: TopicHandlers<T>): () => void {
        const watcher = handlers as Handlers;
        let watchers = this.#watchers.get(topic);
        if (watchers === undefined) {
            watchers = new Set();
            this.#watchers.set(topic, watchers);
            if (this.#live) {
                this.#send({ type: 'sub', topic });
            }
        } else if (this.#snapshots.has(topic)) {
            watcher.snapshot?.(this.#snapshots.get(topic));
        }
        watchers.add(watcher);

        const followed = watchers;
        return () => {
            followed.delete(watcher);
            if (followed.size === 0 && this.#watchers.get(topic) === followed) {
                this.#watchers.delete(topic);
                this.#snapshots.delete(topic);
                if (this.#live) {
                    this.#send({ type: 'unsub', topic });
                }
            }
        };
    }

    async #connect() {
        const generation = ++this.#generation;
        let ticket: string | null;
        try {
            ticket = await this.#options.ticket();
        } catch {
            if (this.#current(generation)) {
                this.#down();
            }
            return;
        }
        if (!this.#current(generation)) {
            return;
        }
        if (ticket === null) {
            this.#refuse();
            return;
        }
        this.#socket = this.#options.connect(ticket, {
            message: (text) => {
                if (this.#current(generation)) {
                    this.#receive(text);
                }
            },
            close: (code) => {
                if (this.#current(generation)) {
                    this.#closed(code);
                }
            },
        });
    }

    #current(generation: number) {
        return this.#running && generation === this.#generation;
    }

    #receive(text: string) {
        const message = parseServerMessage(text);
        if (message === null) {
            return;
        }
        this.#heardAt = Date.now();
        switch (message.type) {
            case 'hello':
                this.#hello(message.contract);
                break;
            case 'snapshot':
                this.#snapshots.set(message.topic, message.data);
                this.#each(message.topic, (watcher) => watcher.snapshot?.(message.data));
                break;
            case 'event':
                this.#each(message.topic, (watcher) => watcher.event?.(message.data));
                break;
            case 'resync':
                // Events of the topic were lost: a new subscription brings a fresh snapshot.
                if (this.#watchers.has(message.topic)) {
                    this.#send({ type: 'sub', topic: message.topic });
                }
                break;
            case 'unsubscribed':
                this.#snapshots.delete(message.topic);
                this.#each(message.topic, (watcher) => watcher.refused?.());
                break;
            case 'pong':
            case 'error':
                break;
        }
    }

    #hello(contract: number) {
        // Another contract means another Observer than the one this page came from: only a reload helps.
        if (contract !== CONTRACT) {
            this.#refuse();
            return;
        }
        const reconnected = this.#wasLive;
        this.#live = true;
        this.#wasLive = true;
        this.#failures = 0;
        this.#clearTimers();
        this.#heartbeat = setInterval(() => this.#beat(), PING_EVERY);
        this.#setState('live');
        for (const topic of this.#watchers.keys()) {
            this.#send({ type: 'sub', topic });
        }
        if (reconnected) {
            this.#options.refetch();
        }
    }

    #beat() {
        if (Date.now() - this.#heardAt >= SILENCE_LIMIT) {
            this.#dropSocket();
            this.#down();
            return;
        }
        this.#send({ type: 'ping' });
    }

    #closed(code: number) {
        this.#socket = null;
        if (code === CLOSE_SESSION_ENDED) {
            // Reading again lets the API say the session ended, which signs the panel out.
            this.#refuse();
            this.#options.refetch();
            return;
        }
        this.#down();
    }

    /** No socket: try again soon, and poll instead if it stays down. */
    #down() {
        this.#live = false;
        this.#snapshots.clear();
        clearInterval(this.#heartbeat);
        this.#heartbeat = undefined;
        if (this.#offline === undefined && this.#poll === undefined) {
            this.#setState('connecting');
            this.#offline = setTimeout(() => {
                this.#offline = undefined;
                this.#setState('offline');
                this.#poll = setInterval(() => {
                    if (document.visibilityState === 'visible') {
                        this.#options.refetch();
                    }
                }, POLL_EVERY);
            }, OFFLINE_AFTER);
        }
        const delay = RETRY_DELAYS[Math.min(this.#failures, RETRY_DELAYS.length - 1)];
        this.#failures += 1;
        clearTimeout(this.#retry);
        this.#retry = setTimeout(() => void this.#connect(), delay);
    }

    #refuse() {
        this.#dropSocket();
        this.#clearTimers();
        this.#setState('refused');
    }

    /** Closes the socket and makes sure nothing it still says is heard. */
    #dropSocket() {
        this.#generation += 1;
        this.#live = false;
        this.#snapshots.clear();
        this.#socket?.close();
        this.#socket = null;
    }

    #clearTimers() {
        clearTimeout(this.#retry);
        clearTimeout(this.#offline);
        clearInterval(this.#poll);
        clearInterval(this.#heartbeat);
        this.#retry = this.#offline = this.#poll = this.#heartbeat = undefined;
    }

    readonly #visibilityChanged = () => {
        if (document.visibilityState === 'hidden') {
            this.#hiddenAt = Date.now();
            return;
        }
        const hiddenAt = this.#hiddenAt;
        this.#hiddenAt = null;
        if (hiddenAt !== null && Date.now() - hiddenAt >= STALE_AFTER) {
            this.#options.refetch();
        }
    };

    #send(message: ClientMessage) {
        this.#socket?.send(JSON.stringify(message));
    }

    #each(topic: Topic, call: (watcher: Handlers) => void) {
        for (const watcher of this.#watchers.get(topic) ?? []) {
            call(watcher);
        }
    }

    #setState(state: LinkState) {
        if (this.#state === state) {
            return;
        }
        this.#state = state;
        for (const listener of this.#listeners) {
            listener();
        }
    }
}
