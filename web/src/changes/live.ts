import { useRef, useState } from 'react';

import type { ChangesLive } from '../api/generated/ChangesLive';
import { useTopic } from '../live/context';

/**
 * The latest start and check of the disk as `server.changes` says them, null until its snapshot and
 * while the link is down. `moved` runs when an event brings a state other than the one held.
 */
export const useChangesLive = (moved?: () => void) => {
    const [live, setLive] = useState<ChangesLive | null>(null);
    const held = useRef('');
    useTopic('server.changes', {
        snapshot: (next) => {
            held.current = JSON.stringify(next);
            setLive(next);
        },
        event: (next) => {
            const text = JSON.stringify(next);
            if (text !== held.current) {
                held.current = text;
                moved?.();
            }
            setLive(next);
        },
        refused: () => setLive(null),
        cleared: () => setLive(null),
    });
    return live;
};
