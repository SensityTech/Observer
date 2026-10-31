import { infiniteQueryOptions, queryOptions } from '@tanstack/react-query';

import { getJson, request, send } from './client';
import type { ChartCategory } from './generated/ChartCategory';
import type { CrashDrops } from './generated/CrashDrops';
import type { DropChanges } from './generated/DropChanges';
import type { DropLog } from './generated/DropLog';
import type { DropSeries } from './generated/DropSeries';
import type { DropsRange } from './generated/DropsRange';
import type { DropsSettings } from './generated/DropsSettings';
import type { DropsSummary } from './generated/DropsSummary';
import type { DropUptime } from './generated/DropUptime';
import type { ResourceTable } from './generated/ResourceTable';
import type { SignatureTable } from './generated/SignatureTable';
import { logSearch, type LogFilters } from '../drops/search';

const PATH = '/servers/default/drops';

/** Every figure of the page is read again this often while it is open. */
export const REFRESH = 30_000;

/** Every drops query: what a change of the retention makes stale. */
export const allDrops = { queryKey: ['drops'] };

const ranged = <T>(name: string, range: DropsRange) =>
    queryOptions({
        queryKey: ['drops', name, range],
        queryFn: ({ signal }) => getJson<T>(`${PATH}/${name}?range=${range}`, signal),
        refetchInterval: REFRESH,
    });

export const summaryQuery = (range: DropsRange) => ranged<DropsSummary>('summary', range);

export const seriesQuery = (range: DropsRange, category: ChartCategory) =>
    queryOptions({
        queryKey: ['drops', 'series', range, category],
        queryFn: ({ signal }) =>
            getJson<DropSeries>(`${PATH}/series?range=${range}&category=${category}`, signal),
        refetchInterval: REFRESH,
    });

export const signaturesQuery = (range: DropsRange) => ranged<SignatureTable>('signatures', range);

export const resourcesQuery = (range: DropsRange) => ranged<ResourceTable>('resources', range);

export const changesQuery = (range: DropsRange) => ranged<DropChanges>('changes', range);

export const uptimeQuery = (range: DropsRange) => ranged<DropUptime>('uptime', range);

/**
 * The drop log under its filters, newest first, a page at a time. Its first page is read again with
 * the rest of the page; once more are loaded it stays as read, as the audit log does, rather than
 * reading every page again each time.
 */
export const logQuery = (filters: LogFilters) =>
    infiniteQueryOptions({
        queryKey: ['drops', 'log', logSearch(filters)],
        queryFn: ({ pageParam, signal }) =>
            getJson<DropLog>(`${PATH}/log${logSearch(filters, pageParam)}`, signal),
        initialPageParam: undefined as string | undefined,
        getNextPageParam: (page) => page.next ?? undefined,
        refetchInterval: (query) => ((query.state.data?.pages.length ?? 0) > 1 ? false : REFRESH),
    });

/** The players a crash of FXServer dropped, by the run it ended: `null` when Observer has none. */
export const crashDropsQuery = (started: string, generation: number) =>
    queryOptions({
        queryKey: ['drops', 'crash', started, generation],
        queryFn: ({ signal }) =>
            getJson<CrashDrops | null>(
                `${PATH}/crash?started=${encodeURIComponent(started)}&generation=${generation}`,
                signal,
            ),
    });

export const dropsSettingsQuery = queryOptions({
    queryKey: ['drops', 'settings'],
    queryFn: ({ signal }) => getJson<DropsSettings>(`${PATH}/settings`, signal),
});

/** The owner's alone, with the password again: the client asks for it when the API wants it. */
export const saveDropsSettings = (settings: DropsSettings) =>
    send<DropsSettings>('PUT', `${PATH}/settings`, settings);

/** The fewest and most days sessions are kept, as Observer checks them. */
export const SESSION_DAYS = { min: 14, max: 365 } as const;

/**
 * Fetches an export and hands it to the browser as a file. A plain link cannot do it: the API wants
 * the session key in a header, which a navigation does not send.
 */
const save = async (path: string, name: string) => {
    const response = await request('GET', path);
    const url = URL.createObjectURL(await response.blob());
    try {
        const link = document.createElement('a');
        link.href = url;
        link.download = name;
        link.click();
    } finally {
        // The click has already handed the file over; a later tick frees it.
        setTimeout(() => URL.revokeObjectURL(url), 0);
    }
};

const pad = (value: number) => String(value).padStart(2, '0');

/** Named after the moment of the export, so successive exports sort and never overwrite each other. */
export const exportName = (what: string, now: Date) =>
    `observer-${what}-${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}.csv`;

/** The log under its filters, every page, as the API writes it; audited there. */
export const downloadLog = (filters: LogFilters) =>
    save(`${PATH}/log.csv${logSearch(filters)}`, exportName('player-drops', new Date()));

/** Drops, play and rates per day from `from` to `to`, both days of the schedule's zone; audited. */
export const downloadRates = (from: string, to: string) =>
    save(`${PATH}/rates.csv?from=${from}&to=${to}`, exportName('drop-rates', new Date()));
