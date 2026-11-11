import { infiniteQueryOptions, queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { Ban } from './generated/Ban';
import type { BanEditRequest } from './generated/BanEditRequest';
import type { BanFilter } from './generated/BanFilter';
import type { BanPage } from './generated/BanPage';
import type { BanRequest } from './generated/BanRequest';
import type { BansSettings } from './generated/BansSettings';
import type { BanTemplate } from './generated/BanTemplate';
import type { LiftRequest } from './generated/LiftRequest';

const PATH = '/servers/default/bans';

/** Every bans query, a profile's included: what a ban, an edit or a lift changes. */
export const allBans = { queryKey: ['bans'] };

/** The list's query string, without the default filter or an empty search. */
export const bansSearch = (filter: BanFilter, search: string, after?: string) => {
    const query = new URLSearchParams();
    if (filter !== 'active') {
        query.set('filter', filter);
    }
    if (search !== '') {
        query.set('search', search);
    }
    if (after !== undefined) {
        query.set('after', after);
    }
    const text = query.toString();
    return text === '' ? '' : `?${text}`;
};

export const bansQuery = (filter: BanFilter, search: string) =>
    infiniteQueryOptions({
        queryKey: ['bans', 'list', filter, search],
        queryFn: ({ pageParam, signal }) =>
            getJson<BanPage>(`${PATH}${bansSearch(filter, search, pageParam)}`, signal),
        initialPageParam: undefined as string | undefined,
        getNextPageParam: (page) => page.next ?? undefined,
    });

export const playerBansQuery = (player: number) =>
    queryOptions({
        queryKey: ['bans', 'player', player],
        queryFn: ({ signal }) => getJson<Ban[]>(`/servers/default/players/${player}/bans`, signal),
    });

export const banPlayer = (request: BanRequest) => send<Ban>('POST', PATH, request);

export const editBan = (code: string, request: BanEditRequest) =>
    send<Ban>('PATCH', `${PATH}/${encodeURIComponent(code)}`, request);

export const liftBan = (code: string, reason: string) =>
    send('POST', `${PATH}/${encodeURIComponent(code)}/lift`, {
        reason: reason.trim() === '' ? null : reason.trim(),
    } satisfies LiftRequest);

/** The templates as whoever bans reads them; the settings hold them too, for who reads those. */
export const banTemplatesQuery = queryOptions({
    queryKey: ['bans', 'templates'],
    queryFn: ({ signal }) => getJson<BanTemplate[]>(`${PATH}/templates`, signal),
});

export const bansSettingsQuery = queryOptions({
    queryKey: ['bans', 'settings'],
    queryFn: ({ signal }) => getJson<BansSettings>(`${PATH}/settings`, signal),
});

export const saveBansSettings = (settings: BansSettings) =>
    send<BansSettings>('PUT', `${PATH}/settings`, settings);

/** What the server counts as the length of a reason or a message: characters, not UTF-16 units. */
export const characters = (text: string) => [...text].length;

/** The refusal message as a player reads it, with the API's placeholders filled in once each. */
export const fillMessage = (
    message: string,
    values: Record<'reason' | 'ends' | 'id' | 'name' | 'by', string>,
) => message.replace(/\{(reason|ends|id|name|by)\}/g, (_, key: keyof typeof values) => values[key]);

export const MAX_REASON = 200;
export const MAX_MESSAGE = 600;
export const MAX_TOKENS = 8;
export const MAX_TEMPLATES = 50;
/** The longest ban short of a permanent one, as the API takes it: ten years. */
export const MAX_MINUTES = 3650 * 24 * 60;
