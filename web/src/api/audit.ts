import { infiniteQueryOptions } from '@tanstack/react-query';

import { getJson, request } from './client';
import type { AuditFilter } from './generated/AuditFilter';
import type { AuditPage } from './generated/AuditPage';

const PAGE_SIZE = 50;

/** The filter as a query string, without the empty fields. */
export const auditSearch = (filter: AuditFilter) => {
    const search = new URLSearchParams();
    for (const [name, value] of Object.entries(filter)) {
        if (value !== undefined && value !== '') {
            search.set(name, String(value));
        }
    }
    const text = search.toString();
    return text === '' ? '' : `?${text}`;
};

export const auditQuery = (filter: AuditFilter) =>
    infiniteQueryOptions({
        queryKey: ['audit', filter],
        queryFn: ({ pageParam, signal }) =>
            getJson<AuditPage>(
                `/audit${auditSearch({ ...filter, before: pageParam, limit: PAGE_SIZE })}`,
                signal,
            ),
        initialPageParam: undefined as number | undefined,
        getNextPageParam: (page) => page.next ?? undefined,
    });

const pad = (value: number) => String(value).padStart(2, '0');

/** Named after the moment of the export, so successive exports sort and never overwrite each other. */
export const exportFilename = (now: Date) =>
    `observer-audit-${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}.csv`;

/**
 * Fetches the export and hands it to the browser as a file. A plain link cannot do it: the API
 * wants the session key in a header, which a navigation does not send.
 */
export const downloadAuditExport = async (filter: AuditFilter) => {
    const response = await request('GET', `/audit/export.csv${auditSearch(filter)}`);
    const url = URL.createObjectURL(await response.blob());
    try {
        const link = document.createElement('a');
        link.href = url;
        link.download = exportFilename(new Date());
        link.click();
    } finally {
        // The click has already handed the file over; a later tick frees it.
        setTimeout(() => URL.revokeObjectURL(url), 0);
    }
};
