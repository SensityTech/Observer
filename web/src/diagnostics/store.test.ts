import { describe, expect, it } from 'vitest';

import { CONHOST, CRASH, HANG, MYSQL, SHELL, SNAPSHOT } from './fixtures';
import { applyEvent, MAX_CRASHES, MAX_PROCESSES } from './store';

describe('applyEvent', () => {
    it('should put an updated process in place of its row', () => {
        const ended = { ...SHELL, endedAt: '2026-10-29T11:07:00Z' };

        const next = applyEvent(SNAPSHOT, { kind: 'process', process: ended });

        expect(next.processes).toEqual([ended, CONHOST]);
    });

    it('should add a new process first and keep the newest 200', () => {
        const full = {
            ...SNAPSHOT,
            processes: Array.from({ length: MAX_PROCESSES }, (_, id) => ({ ...CONHOST, id })),
        };
        const started = { ...SHELL, id: 900, lastStartedAt: '2026-10-29T12:00:00Z' };

        const next = applyEvent(full, { kind: 'process', process: started });

        expect(next.processes).toHaveLength(MAX_PROCESSES);
        expect(next.processes[0]).toBe(started);
    });

    it('should order crashes by time and keep the newest 50', () => {
        const older = { ...CRASH, id: 40, at: '2026-10-28T09:00:00Z' };
        const full = {
            ...SNAPSHOT,
            crashes: Array.from({ length: MAX_CRASHES }, (_, id) => ({ ...HANG, id })),
        };

        expect(
            applyEvent(SNAPSHOT, { kind: 'crash', crash: older }).crashes.map((crash) => crash.id),
        ).toEqual([3, 2, 40]);
        expect(applyEvent(full, { kind: 'crash', crash: { ...CRASH, id: 99 } }).crashes).toHaveLength(
            MAX_CRASHES,
        );
    });

    it('should replace the connections and the crash folder whole', () => {
        const closed = { ...MYSQL, open: 0 };
        const folder = { ...SNAPSHOT.crashFolder, files: 0, bytes: 0 };

        const next = applyEvent(
            applyEvent(SNAPSHOT, { kind: 'connections', destinations: [closed], listeners: [] }),
            {
                kind: 'crashFolder',
                folder,
            },
        );

        expect(next.destinations).toEqual([closed]);
        expect(next.listeners).toEqual([]);
        expect(next.crashFolder).toBe(folder);
        expect(next.processes).toBe(SNAPSHOT.processes);
    });
});
