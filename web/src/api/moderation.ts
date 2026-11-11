import { queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { AnnouncementRequest } from './generated/AnnouncementRequest';
import type { DirectMessageRequest } from './generated/DirectMessageRequest';
import type { KickAllRequest } from './generated/KickAllRequest';
import type { KickedAll } from './generated/KickedAll';
import type { KickRequest } from './generated/KickRequest';
import type { KickSettings } from './generated/KickSettings';

const PATH = '/servers/default';

/** A connected player an action addresses: their server ID, and who the panel showed under it. */
export type Target = { netId: number; name: string; license: string | null };

export const kickPlayer = (target: Target, reason: string) =>
    send('POST', `${PATH}/kicks`, {
        player: target.netId,
        license: target.license,
        reason,
    } satisfies KickRequest);

export const kickEveryone = (reason: string) =>
    send<KickedAll>('POST', `${PATH}/kicks/all`, { reason } satisfies KickAllRequest);

export const announce = (message: string) =>
    send('POST', `${PATH}/announcements`, { message } satisfies AnnouncementRequest);

export const messagePlayer = (target: Target, message: string) =>
    send('POST', `${PATH}/messages`, {
        player: target.netId,
        license: target.license,
        message,
    } satisfies DirectMessageRequest);

export const kickSettingsQuery = queryOptions({
    queryKey: ['kicks', 'settings'],
    queryFn: ({ signal }) => getJson<KickSettings>(`${PATH}/kicks/settings`, signal),
});

export const saveKickSettings = (settings: KickSettings) =>
    send<KickSettings>('PUT', `${PATH}/kicks/settings`, settings);

/** The kick message as a player reads it, with the API's placeholders filled in once each. */
export const fillKickMessage = (message: string, values: Record<'reason' | 'by', string>) =>
    message.replace(/\{(reason|by)\}/g, (_, key: keyof typeof values) => values[key]);

export const MAX_KICK_REASON = 200;
export const MAX_KICK_MESSAGE = 600;
/** The longest announcement or direct message, in characters. */
export const MAX_TEXT = 400;
