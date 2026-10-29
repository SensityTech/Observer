import { useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef } from 'react';

import type { ServerState } from '../api/generated/ServerState';
import { useServerStatus } from '../server/context';

/**
 * How long after the server's state or run changes the statistics are read again: the run's row,
 * which the charts' markers come from, is written once the sampler has taken in the supervisor's
 * event.
 */
export const REFETCH_AFTER_MS = 1000;

type Run = { state: ServerState; generation: number };

/**
 * Calls `refetch` once the server's state or run has held for `delay`: a change within that wait
 * starts it over. The first status seen is how things stand, not a change.
 */
export const refetchOnChange = (refetch: () => void, delay = REFETCH_AFTER_MS) => {
    let previous: Run | null = null;
    let timer: ReturnType<typeof setTimeout> | undefined;
    return {
        seen({ state, generation }: Run) {
            const changed =
                previous !== null && (previous.state !== state || previous.generation !== generation);
            previous = { state, generation };
            if (changed) {
                clearTimeout(timer);
                timer = setTimeout(refetch, delay);
            }
        },
        stop() {
            clearTimeout(timer);
        },
    };
};

/** Every statistics query read again after a start, a stop or a crash, so markers show at once. */
export function useRefetchOnChange() {
    const client = useQueryClient();
    const { status } = useServerStatus();
    const watcher = useRef<ReturnType<typeof refetchOnChange> | null>(null);
    useEffect(() => {
        const created = refetchOnChange(() => void client.invalidateQueries({ queryKey: ['metrics'] }));
        watcher.current = created;
        return () => {
            created.stop();
            watcher.current = null;
        };
    }, [client]);
    useEffect(() => {
        if (status !== null) {
            watcher.current?.seen(status);
        }
    }, [status]);
}
