import { describe, expect, it, vi } from 'vitest';

import type { ConsoleLine } from '../api/generated/ConsoleLine';
import { ConsoleStore, type Row } from './store';

const ESC = String.fromCodePoint(0x1b);

const line = (seq: number, text = `line ${seq}`, kind: ConsoleLine[3] = 'stdout'): ConsoleLine => [
    seq,
    1_761_642_000_000 + seq,
    1,
    kind,
    null,
    text,
    0,
];

/** A store whose frames run when the test says. */
const setup = (limit?: number) => {
    const frames: (() => void)[] = [];
    const store = new ConsoleStore({ limit, schedule: (run) => frames.push(run) });
    const frame = () => frames.splice(0).forEach((run) => run());
    return { store, frame };
};

const seqs = (rows: readonly Row[]) => rows.map((row) => (row.type === 'line' ? row.seq : 'gap'));

describe('ConsoleStore', () => {
    it('should keep the lines in order, parsed once, and resume after the last one', () => {
        const { store, frame } = setup();

        store.snapshot({ lines: [line(1), line(2, `${ESC}[91mboom${ESC}[0m`, 'stderr')], gap: false });
        store.append([line(3)]);
        frame();

        expect(seqs(store.getRows())).toEqual([1, 2, 3]);
        expect(store.since()).toBe(3);
        expect(store.getRows()[1]).toMatchObject({
            kind: 'stderr',
            spans: [{ text: 'boom', tone: 'red', bold: false }],
            search: 'boom',
        });
        expect(store.received()).toBe(3);
        expect(store.errors()).toBe(1);
    });

    it('should tell its listeners once per frame however many batches came', () => {
        const { store, frame } = setup();
        const listener = vi.fn();
        store.subscribe(listener);

        store.append([line(1)]);
        store.append([line(2)]);
        store.append([line(3)]);

        expect(listener).not.toHaveBeenCalled();
        expect(store.getRows()).toEqual([]);
        frame();
        expect(listener).toHaveBeenCalledTimes(1);
        expect(seqs(store.getRows())).toEqual([1, 2, 3]);
    });

    it('should drop the lines it already holds when a resumed snapshot repeats them', () => {
        const { store, frame } = setup();
        store.snapshot({ lines: [line(1), line(2)], gap: false });

        store.snapshot({ lines: [line(2), line(3)], gap: false });
        store.append([line(3), line(4)]);
        frame();

        expect(seqs(store.getRows())).toEqual([1, 2, 3, 4]);
    });

    it('should mark where lines are missing after a resumption that came too late', () => {
        const { store, frame } = setup();
        store.snapshot({ lines: [line(1), line(2)], gap: false });

        store.snapshot({ lines: [line(40), line(41)], gap: true });
        frame();

        expect(seqs(store.getRows())).toEqual([1, 2, 'gap', 40, 41]);
        expect(store.since()).toBe(41);
    });

    it('should start over when the lines come from another Observer', () => {
        const { store, frame } = setup();
        store.snapshot({ lines: [line(900), line(901)], gap: false });

        // An Observer started since, numbering from below what the panel holds.
        store.snapshot({ lines: [line(5), line(6)], gap: true });
        frame();

        expect(seqs(store.getRows())).toEqual([5, 6]);
    });

    it('should keep only the newest lines past its limit', () => {
        const { store, frame } = setup(3);

        store.append([line(1), line(2), line(3)]);
        store.append([line(4), line(5)]);
        frame();

        expect(seqs(store.getRows())).toEqual([3, 4, 5]);
        expect(store.received()).toBe(5);
    });
});
