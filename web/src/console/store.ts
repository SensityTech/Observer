import type { ConsoleLine } from '../api/generated/ConsoleLine';
import type { ConsoleSnapshot } from '../api/generated/ConsoleSnapshot';
import type { LineKind } from '../api/generated/LineKind';
import { parseSgr, plainText, type Span } from './sgr';

/** The lines the panel keeps, DESIGN.md §8.11: the newest go in, the oldest out. */
export const LIMIT = 5_000;

export type LineRow = {
    type: 'line';
    seq: number;
    /** Milliseconds since the epoch. */
    at: number;
    kind: LineKind;
    actor: string | null;
    spans: Span[];
    /** The text without colours, lower-cased, for the search. */
    search: string;
};

/** Where lines are missing: they left Observer's memory before the panel got them. */
export type GapRow = { type: 'gap'; seq: number };

export type Row = LineRow | GapRow;

type StoreOptions = {
    limit?: number;
    /** Runs the listeners once per frame at most, however many batches came in it. */
    schedule?: (run: () => void) => void;
};

const toRow = ([seq, at, , kind, actor, text]: ConsoleLine): LineRow => ({
    type: 'line',
    seq,
    at,
    kind,
    actor,
    spans: parseSgr(text),
    search: plainText(text).toLowerCase(),
});

/**
 * The console as the panel holds it, fed by the `console` topic and read by the page through
 * `useSyncExternalStore`. It outlives the page's subscription: when the link comes back, the topic
 * resumes after the last line held, and Observer says if any are missing.
 */
export class ConsoleStore {
    readonly #limit: number;
    readonly #schedule: (run: () => void) => void;
    readonly #listeners = new Set<() => void>();
    #rows: Row[] = [];
    /** What the listeners were last told, so that a frame renders once. */
    #published: Row[] = [];
    #scheduled = false;
    #last: number | null = null;
    #received = 0;
    #errors = 0;

    constructor({ limit = LIMIT, schedule = (run) => requestAnimationFrame(run) }: StoreOptions = {}) {
        this.#limit = limit;
        this.#schedule = schedule;
    }

    subscribe = (listener: () => void) => {
        this.#listeners.add(listener);
        return () => {
            this.#listeners.delete(listener);
        };
    };

    getRows = (): readonly Row[] => this.#published;

    /** The number of the last line held, which a new subscription resumes after. */
    since = () => this.#last;

    /** How many lines came in since the store was made, the ones gone out again included. */
    received = () => this.#received;

    /** How many of them were stderr. */
    errors = () => this.#errors;

    snapshot({ lines, gap }: ConsoleSnapshot) {
        const first = lines.at(0);
        if (gap && first !== undefined && this.#last !== null && first[0] <= this.#last) {
            // Numbers below the panel's own: another Observer's, which keeps none of these lines.
            this.#rows = [];
            this.#last = null;
        }
        if (gap && this.#rows.length > 0) {
            this.#rows = [...this.#rows, { type: 'gap', seq: (this.#last ?? 0) + 0.5 }];
        }
        this.append(lines);
    }

    append(lines: ConsoleLine[]) {
        const last = this.#last;
        const fresh = last === null ? lines : lines.filter(([seq]) => seq > last);
        if (fresh.length === 0) {
            if (this.#rows !== this.#published) {
                this.#notify();
            }
            return;
        }
        const rows = fresh.map(toRow);
        this.#rows = [...this.#rows, ...rows].slice(-this.#limit);
        this.#last = fresh.at(-1)?.[0] ?? last;
        this.#received += rows.length;
        this.#errors += rows.filter((row) => row.kind === 'stderr').length;
        this.#notify();
    }

    #notify() {
        if (this.#scheduled) {
            return;
        }
        this.#scheduled = true;
        this.#schedule(() => {
            this.#scheduled = false;
            this.#published = this.#rows;
            for (const listener of this.#listeners) {
                listener();
            }
        });
    }
}
