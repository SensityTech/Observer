import { queryOptions } from '@tanstack/react-query';

import { getJson } from './client';
import type { AccountSession } from './generated/AccountSession';
import type { PermissionInfo } from './generated/PermissionInfo';
import type { User } from './generated/User';

export const usersQuery = queryOptions({
    queryKey: ['users'],
    queryFn: ({ signal }) => getJson<User[]>('/users', signal),
});

/** The catalog only changes with Observer's version. */
export const permissionCatalogQuery = queryOptions({
    queryKey: ['permissions'],
    queryFn: ({ signal }) => getJson<PermissionInfo[]>('/permissions', signal),
    staleTime: Infinity,
});

export const sessionsQuery = queryOptions({
    queryKey: ['sessions'],
    queryFn: ({ signal }) => getJson<AccountSession[]>('/account/sessions', signal),
});
