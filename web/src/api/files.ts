import { queryOptions } from '@tanstack/react-query';

import { getJson } from './client';
import type { FileAccesses } from './generated/FileAccesses';
import type { FileGroup } from './generated/FileGroup';
import type { MyFileAccess } from './generated/MyFileAccess';

export const fileGroupsQuery = queryOptions({
    queryKey: ['files', 'groups'],
    queryFn: ({ signal }) => getJson<FileGroup[]>('/servers/default/files/groups', signal),
});

export const fileAccessesQuery = queryOptions({
    queryKey: ['files', 'accesses'],
    queryFn: ({ signal }) => getJson<FileAccesses>('/files/accesses', signal),
});

export const myFileAccessQuery = queryOptions({
    queryKey: ['account', 'files'],
    queryFn: ({ signal }) => getJson<MyFileAccess>('/account/files', signal),
});
