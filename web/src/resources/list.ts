import type { ResourceOrigin } from '../api/generated/ResourceOrigin';
import type { ResourceRow } from '../api/generated/ResourceRow';

export const FILTERS = ['all', 'started', 'stopped', 'failed', 'warned'] as const;
export type Filter = (typeof FILTERS)[number];

/** FXServer said it could not start the resource, and has not started it since. */
export const didNotStart = (row: ResourceRow) => row.failedAt !== null && row.state !== 'started';

/**
 * Whether a resource shows under a filter. One FXServer could not start is not counted with the
 * stopped ones: someone stopped those, or nothing ever started them.
 */
export const passes = (row: ResourceRow, filter: Filter) => {
    switch (filter) {
        case 'all':
            return true;
        case 'started':
            return row.state === 'started';
        case 'stopped':
            return row.state === 'stopped' && !didNotStart(row);
        case 'failed':
            return didNotStart(row);
        case 'warned':
            return row.warnings !== null;
    }
};

/** How many resources each filter shows. */
export const countByFilter = (rows: readonly ResourceRow[]): Record<Filter, number> => {
    const counts: Record<Filter, number> = { all: 0, started: 0, stopped: 0, failed: 0, warned: 0 };
    for (const row of rows) {
        for (const filter of FILTERS) {
            if (passes(row, filter)) {
                counts[filter] += 1;
            }
        }
    }
    return counts;
};

/** The resources of one folder. */
export type Group = {
    /** What tells the group from the others, for a list's key and a remembered fold. */
    key: string;
    /** The folder as it reads on disk: `resources`, `[core]/[ox]`, `system_resources`. */
    folder: string;
    origin: ResourceOrigin;
    /** Those the filter and the search keep, in name order. */
    rows: ResourceRow[];
    /** Of every resource of the folder, whatever is shown. */
    started: number;
    total: number;
};

/** The folder that holds the build's own resources, and the server's at their root. */
const ROOTS: Record<ResourceOrigin, string> = { server: 'resources', build: 'system_resources' };

const folderOf = (row: ResourceRow) =>
    row.origin === 'build'
        ? row.folder === ''
            ? ROOTS.build
            : `${ROOTS.build}/${row.folder}`
        : row.folder === ''
          ? ROOTS.server
          : row.folder;

/**
 * The resources by folder, as the filter and the search leave them: the root of the server's
 * `resources` first, its categories by name, then the build's own. A folder left with nothing to
 * show is left out.
 */
export const groupByFolder = (rows: readonly ResourceRow[], filter: Filter, find: string): Group[] => {
    const needle = find.trim().toLowerCase();
    const groups = new Map<string, Group>();
    for (const row of rows) {
        const key = `${row.origin}:${row.folder}`;
        let group = groups.get(key);
        if (group === undefined) {
            group = { key, folder: folderOf(row), origin: row.origin, rows: [], started: 0, total: 0 };
            groups.set(key, group);
        }
        group.total += 1;
        group.started += row.state === 'started' ? 1 : 0;
        if (passes(row, filter) && row.name.toLowerCase().includes(needle)) {
            group.rows.push(row);
        }
    }
    const rank = (group: Group) => (group.origin === 'build' ? 2 : group.folder === ROOTS.server ? 0 : 1);
    return [...groups.values()]
        .filter((group) => group.rows.length > 0)
        .toSorted(
            (a, b) => rank(a) - rank(b) || a.folder.localeCompare(b.folder, 'en', { sensitivity: 'base' }),
        );
};
