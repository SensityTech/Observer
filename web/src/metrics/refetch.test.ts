import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { refetchOnChange } from './refetch';

describe('refetchOnChange', () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it('should read the statistics again a second after the state changes, not on the first status', () => {
        const refetch = vi.fn();
        const watcher = refetchOnChange(refetch);

        watcher.seen({ state: 'online', generation: 1 });
        vi.advanceTimersByTime(5000);
        expect(refetch).not.toHaveBeenCalled();

        watcher.seen({ state: 'online', generation: 1 });
        watcher.seen({ state: 'crashed', generation: 1 });
        vi.advanceTimersByTime(999);
        expect(refetch).not.toHaveBeenCalled();
        vi.advanceTimersByTime(1);
        expect(refetch).toHaveBeenCalledTimes(1);
    });

    it('should start the wait over when another change comes within it', () => {
        const refetch = vi.fn();
        const watcher = refetchOnChange(refetch);
        watcher.seen({ state: 'crashed', generation: 1 });

        watcher.seen({ state: 'starting', generation: 2 });
        vi.advanceTimersByTime(600);
        watcher.seen({ state: 'online', generation: 2 });
        vi.advanceTimersByTime(600);
        expect(refetch).not.toHaveBeenCalled();
        vi.advanceTimersByTime(400);
        expect(refetch).toHaveBeenCalledTimes(1);
    });

    it('should count a new run as a change even in the same state, and nothing once stopped', () => {
        const refetch = vi.fn();
        const watcher = refetchOnChange(refetch);
        watcher.seen({ state: 'online', generation: 1 });

        watcher.seen({ state: 'online', generation: 2 });
        vi.advanceTimersByTime(1000);
        expect(refetch).toHaveBeenCalledTimes(1);

        watcher.seen({ state: 'stopped', generation: 2 });
        watcher.stop();
        vi.advanceTimersByTime(1000);
        expect(refetch).toHaveBeenCalledTimes(1);
    });
});
