import { useQueryClient } from '@tanstack/react-query';
import { useRef, useState } from 'react';

import { allEntities } from '../api/entities';
import type { EntitiesLive } from '../api/generated/EntitiesLive';
import { useTopic } from '../live/context';

/**
 * The state of the list while the component is mounted, or null while the link is down. Following
 * it is what keeps the server listing its entities; each new sweep asks the list again.
 */
export const useEntitiesLive = () => {
    const client = useQueryClient();
    const [live, setLive] = useState<EntitiesLive | null>(null);
    const seen = useRef<string | null>(null);
    const took = (next: EntitiesLive) => {
        setLive(next);
        const sweep = `${next.run ?? ''}:${next.sweep}`;
        // The first state is no newer than the list the page asks for as it opens.
        if (seen.current !== null && seen.current !== sweep) {
            void client.invalidateQueries(allEntities);
        }
        seen.current = sweep;
    };
    const lost = () => setLive(null);
    useTopic('server.entities', { snapshot: took, event: took, refused: lost, cleared: lost });
    return live;
};
