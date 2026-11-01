import { infiniteQueryOptions, queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { ChangesSettings } from './generated/ChangesSettings';
import type { ChangesSettingsView } from './generated/ChangesSettingsView';
import type { Comparison } from './generated/Comparison';
import type { FileDiff } from './generated/FileDiff';
import type { FileText } from './generated/FileText';
import type { Purged } from './generated/Purged';
import type { ResourceDetail } from './generated/ResourceDetail';
import type { ResourceVersion } from './generated/ResourceVersion';
import type { StartList } from './generated/StartList';

const PATH = '/servers/default/changes';

/** A start by its id, or `disk` for the latest check of the disk. */
export type ComparedTo = number | 'disk';

/** The query string of the defined values, in order. */
const query = (values: Record<string, string | number | undefined>) => {
    const search = new URLSearchParams();
    for (const [name, value] of Object.entries(values)) {
        if (value !== undefined) {
            search.set(name, String(value));
        }
    }
    const text = search.toString();
    return text === '' ? '' : `?${text}`;
};

/**
 * The newest page of starts, with the latest check of the disk: what moves while a snapshot is
 * taken, read again on its own.
 */
export const startsQuery = queryOptions({
    queryKey: ['changes', 'starts'],
    queryFn: ({ signal }) => getJson<StartList>(`${PATH}/starts`, signal),
});

/** The starts older than `before`, the newest page's `next`, a page at a time. */
export const olderStartsQuery = (before: number) =>
    infiniteQueryOptions({
        queryKey: ['changes', 'starts', 'older', before],
        queryFn: ({ pageParam, signal }) =>
            getJson<StartList>(`${PATH}/starts${query({ before: pageParam })}`, signal),
        initialPageParam: before,
        getNextPageParam: (page) => page.next ?? undefined,
    });

/** Two snapshots compared; without `to` the latest start, without `from` the start before `to`. */
export const comparisonQuery = (from: number | undefined, to: ComparedTo | undefined) =>
    queryOptions({
        queryKey: ['changes', 'comparison', from ?? null, to ?? null],
        queryFn: ({ signal }) => getJson<Comparison>(`${PATH}${query({ from, to })}`, signal),
    });

export const resourceQuery = (from: number | undefined, to: ComparedTo | undefined, path: string) =>
    queryOptions({
        queryKey: ['changes', 'resource', from ?? null, to ?? null, path],
        queryFn: ({ signal }) =>
            getJson<ResourceDetail>(`${PATH}/resource${query({ from, to, path })}`, signal),
    });

export const historyQuery = (name: string) =>
    queryOptions({
        queryKey: ['changes', 'history', name],
        queryFn: ({ signal }) => getJson<ResourceVersion[]>(`${PATH}/history${query({ name })}`, signal),
    });

export type DiffRequest = {
    from: number;
    to: number;
    path: string;
    /** The file's path in `from`, when it or its resource moved. */
    fromPath: string | null;
    page: number;
    /** Unchanged lines around each change. */
    context: number;
    ignoreWhitespace: boolean;
};

/** The unchanged lines a diff shows around each change, until more are asked for. */
export const DEFAULT_CONTEXT = 3;

// Two snapshots' contents never change once listed: a diff or a file is read once.
export const diffQuery = (request: DiffRequest) =>
    queryOptions({
        queryKey: ['changes', 'diff', request],
        queryFn: ({ signal }) =>
            getJson<FileDiff>(
                `${PATH}/diff${query({
                    from: request.from,
                    to: request.to,
                    path: request.path,
                    fromPath: request.fromPath ?? undefined,
                    page: request.page,
                    context: request.context,
                    whitespace: request.ignoreWhitespace ? 'ignore' : 'show',
                })}`,
                signal,
            ),
        staleTime: Infinity,
        retry: false,
    });

export const fileQuery = (start: number, path: string, page: number) =>
    queryOptions({
        queryKey: ['changes', 'file', start, path, page],
        queryFn: ({ signal }) => getJson<FileText>(`${PATH}/file${query({ start, path, page })}`, signal),
        staleTime: Infinity,
        retry: false,
    });

export const changesSettingsQuery = queryOptions({
    queryKey: ['changes', 'settings'],
    queryFn: ({ signal }) => getJson<ChangesSettingsView>(`${PATH}/settings`, signal),
});

/** Lists and hashes the disk now; refused with `check_running` or `check_too_soon`. */
export const checkDisk = () => send('POST', `${PATH}/check`);
export const saveChangesSettings = (settings: ChangesSettings) =>
    send<ChangesSettingsView>('PUT', `${PATH}/settings`, settings);
export const verifyFiles = () => send('POST', `${PATH}/verify`);
/** Clears the kept contents of every version the path had, in any start or check. */
export const purgeContents = (path: string) => send<Purged>('DELETE', `${PATH}/contents${query({ path })}`);
