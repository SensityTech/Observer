import { queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { AddPass } from './generated/AddPass';
import type { MaintenancePass } from './generated/MaintenancePass';
import type { MaintenancePasses } from './generated/MaintenancePasses';
import type { MaintenancePreview } from './generated/MaintenancePreview';
import type { MaintenanceSettings } from './generated/MaintenanceSettings';
import type { SetMaintenance } from './generated/SetMaintenance';

const PATH = '/servers/default/maintenance';

/** What whoever turns maintenance on is shown first: the message players will read. */
export const maintenancePreviewQuery = queryOptions({
    queryKey: ['maintenance', 'preview'],
    queryFn: ({ signal }) => getJson<MaintenancePreview>(PATH, signal),
});

/** Closes the server to players or opens it again; the `server.admission` topic tells the rest. */
export const setMaintenance = (on: boolean) => send('PUT', PATH, { on } satisfies SetMaintenance);

export const maintenanceSettingsQuery = queryOptions({
    queryKey: ['maintenance', 'settings'],
    queryFn: ({ signal }) => getJson<MaintenanceSettings>(`${PATH}/settings`, signal),
});

export const saveMaintenanceSettings = (settings: MaintenanceSettings) =>
    send<MaintenanceSettings>('PUT', `${PATH}/settings`, settings);

/** The players who still get in during maintenance. */
export const maintenancePassesQuery = queryOptions({
    queryKey: ['maintenance', 'passes'],
    queryFn: ({ signal }) => getJson<MaintenancePasses>(`${PATH}/passes`, signal),
});

export const addPass = (license: string, note: string) =>
    send<MaintenancePass>('POST', `${PATH}/passes`, {
        license: license.trim(),
        note: note.trim() === '' ? null : note.trim(),
    } satisfies AddPass);

export const removePass = (id: number) => send('DELETE', `${PATH}/passes/${id}`);
