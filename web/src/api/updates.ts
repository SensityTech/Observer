import { queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { InstallUpdate } from './generated/InstallUpdate';
import type { UpdatesView } from './generated/UpdatesView';

/** The newest release and the install under way: the owner's alone. */
export const updatesQuery = queryOptions({
    queryKey: ['updates'],
    queryFn: ({ signal }) => getJson<UpdatesView>('/updates', signal),
    retry: false,
});

/** Answered once GitHub has. */
export const checkUpdates = () => send<UpdatesView>('POST', '/updates/check');

export const installUpdate = (request: InstallUpdate) =>
    send<UpdatesView>('POST', '/updates/install', request);

export const cancelUpdate = () => send<UpdatesView>('DELETE', '/updates/install');

/** An install under way, which the panel follows closely. */
export const isUpdating = (view: UpdatesView | undefined) => view?.job?.state === 'running';
