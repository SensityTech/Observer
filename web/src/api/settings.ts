import { queryOptions } from '@tanstack/react-query';

import { getJson } from './client';
import type { ObserverSettings } from './generated/ObserverSettings';

/** What `observer.toml` said at start, and how long the statistics stay: the owner's to read. */
export const observerSettingsQuery = queryOptions({
    queryKey: ['settings', 'observer'],
    queryFn: ({ signal }) => getJson<ObserverSettings>('/settings/observer', signal),
    // Read once at start: nothing changes them while Observer runs.
    staleTime: Infinity,
});
