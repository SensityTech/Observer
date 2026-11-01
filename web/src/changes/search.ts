import type { ComparedTo } from '../api/changes';

/** `/changes?from=&to=`: the two snapshots compared, the API's defaults when left out. */
export type ChangesSearch = { from?: number; to?: ComparedTo };

/** `/changes/resource?from=&to=&path=`: one resource of a comparison. */
export type ResourceSearch = ChangesSearch & { path: string };

// The address bar is the user's to edit: what is not an id is dropped rather than sent to the API.

const startId = (value: unknown) =>
    typeof value === 'number' && Number.isSafeInteger(value) && value > 0 ? value : undefined;

export const changesSearch = (search: Record<string, unknown>): ChangesSearch => ({
    from: startId(search.from),
    to: search.to === 'disk' ? 'disk' : startId(search.to),
});

export const resourceSearch = (search: Record<string, unknown>): ResourceSearch => ({
    ...changesSearch(search),
    path: typeof search.path === 'string' ? search.path : '',
});
