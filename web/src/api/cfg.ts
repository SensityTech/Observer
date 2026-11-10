import { queryOptions } from '@tanstack/react-query';

import { getJson, request, send } from './client';
import type { CfgDocument } from './generated/CfgDocument';
import type { CfgDraft } from './generated/CfgDraft';
import type { CfgRestore } from './generated/CfgRestore';
import type { CfgReview } from './generated/CfgReview';
import type { CfgSave } from './generated/CfgSave';
import type { CfgVersion } from './generated/CfgVersion';
import type { TextDiff } from './generated/TextDiff';

const PATH = '/servers/default/cfg';

/**
 * The launch's cfg, its secrets as marks. Read once for an edit: a save names the file it started
 * from, and is refused when the file is another by then.
 */
export const cfgQuery = queryOptions({
    queryKey: ['cfg'],
    queryFn: ({ signal }) => getJson<CfgDocument>(PATH, signal),
    staleTime: Infinity,
    refetchOnWindowFocus: false,
    retry: false,
});

const draft = async <T>(path: string, text: string, signal: AbortSignal) =>
    (await (await request('POST', path, { body: { text } satisfies CfgDraft, signal })).json()) as T;

/** What a text of the cfg sets and starts and what its scan finds. Nothing is written. */
export const cfgReviewQuery = (text: string) =>
    queryOptions({
        queryKey: ['cfg', 'review', text],
        queryFn: ({ signal }) => draft<CfgReview>(`${PATH}/check`, text, signal),
        staleTime: Infinity,
        gcTime: 0,
        retry: false,
    });

/** The lines a save of the text would change in the file. Nothing is written. */
export const cfgDiffQuery = (text: string) =>
    queryOptions({
        queryKey: ['cfg', 'diff', text],
        queryFn: ({ signal }) => draft<TextDiff>(`${PATH}/diff`, text, signal),
        staleTime: Infinity,
        gcTime: 0,
        retry: false,
    });

/** Refused with `cfg_changed` when the file is no longer the one `base` names. */
export const saveCfg = (save: CfgSave) => send<CfgDocument>('PUT', PATH, save);

export const cfgVersionsQuery = queryOptions({
    queryKey: ['cfg', 'versions'],
    queryFn: ({ signal }) => getJson<CfgVersion[]>(`${PATH}/versions`, signal),
});

/** The lines putting a version back would change in the file. */
export const cfgVersionDiffQuery = (id: number, base: string) =>
    queryOptions({
        queryKey: ['cfg', 'versions', id, base],
        queryFn: ({ signal }) => getJson<TextDiff>(`${PATH}/versions/${id}`, signal),
        staleTime: Infinity,
        retry: false,
    });

export const restoreCfg = (id: number, restore: CfgRestore) =>
    send<CfgDocument>('POST', `${PATH}/versions/${id}/restore`, restore);
