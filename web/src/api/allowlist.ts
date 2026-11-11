import { infiniteQueryOptions, queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { AllowlistSettings } from './generated/AllowlistSettings';
import type { Approval } from './generated/Approval';
import type { ApprovedPage } from './generated/ApprovedPage';
import type { ApproveRequest } from './generated/ApproveRequest';
import type { DeniedRequests } from './generated/DeniedRequests';
import type { DenyRequests } from './generated/DenyRequests';
import type { RequestPage } from './generated/RequestPage';

const PATH = '/servers/default/allowlist';

/** Every query of the two lists: what an answer, an approval or a removal changes. */
export const allowlistLists = { queryKey: ['allowlist', 'lists'] };

/** A list's query string, without an empty search. */
export const allowlistSearch = (search: string, after?: string) => {
    const query = new URLSearchParams();
    if (search !== '') {
        query.set('search', search);
    }
    if (after !== undefined) {
        query.set('after', after);
    }
    const text = query.toString();
    return text === '' ? '' : `?${text}`;
};

export const requestsQuery = (search: string) =>
    infiniteQueryOptions({
        queryKey: ['allowlist', 'lists', 'requests', search],
        queryFn: ({ pageParam, signal }) =>
            getJson<RequestPage>(`${PATH}/requests${allowlistSearch(search, pageParam)}`, signal),
        initialPageParam: undefined as string | undefined,
        getNextPageParam: (page) => page.next ?? undefined,
    });

export const approvedQuery = (search: string) =>
    infiniteQueryOptions({
        queryKey: ['allowlist', 'lists', 'approved', search],
        queryFn: ({ pageParam, signal }) =>
            getJson<ApprovedPage>(`${PATH}/approved${allowlistSearch(search, pageParam)}`, signal),
        initialPageParam: undefined as string | undefined,
        getNextPageParam: (page) => page.next ?? undefined,
    });

export const approveRequest = (code: string) =>
    send<Approval>('POST', `${PATH}/requests/${encodeURIComponent(code)}/approve`);

export const denyRequest = (code: string) => send('DELETE', `${PATH}/requests/${encodeURIComponent(code)}`);

/** Denies every request last tried at `until` or before: those made since stay. */
export const denyRequests = (until: string) =>
    send<DeniedRequests>('POST', `${PATH}/requests/deny`, { until } satisfies DenyRequests);

export const approveLicence = (license: string, note: string) =>
    send<Approval>('POST', `${PATH}/approved`, {
        license: license.trim(),
        note: note.trim() === '' ? null : note.trim(),
    } satisfies ApproveRequest);

export const removeApproval = (id: number) => send('DELETE', `${PATH}/approved/${id}`);

export const allowlistSettingsQuery = queryOptions({
    queryKey: ['allowlist', 'settings'],
    queryFn: ({ signal }) => getJson<AllowlistSettings>(`${PATH}/settings`, signal),
});

export const saveAllowlistSettings = (settings: AllowlistSettings) =>
    send<AllowlistSettings>('PUT', `${PATH}/settings`, settings);

export const MAX_NOTE = 64;
