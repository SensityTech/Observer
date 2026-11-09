import { queryOptions } from '@tanstack/react-query';

import { getJson } from './client';
import type { CfgConvar } from './generated/CfgConvar';
import type { CfgConvars } from './generated/CfgConvars';
import type { Convar } from './generated/Convar';
import type { ConvarCatalog } from './generated/ConvarCatalog';

const listing = (value: unknown, key: string) =>
    typeof value === 'object' &&
    value !== null &&
    key in value &&
    Array.isArray((value as Record<string, unknown>)[key]);

/** The convars FXServer declares, as this build of Observer carries them: it changes with Observer only. */
export const convarCatalogQuery = queryOptions({
    queryKey: ['convars'],
    queryFn: async ({ signal }) => {
        const body = await getJson<unknown>('/convars', signal);
        return listing(body, 'convars') ? (body as ConvarCatalog) : null;
    },
    staleTime: Infinity,
});

/** What the saved launch's cfg sets, null until the launch is saved. */
export const cfgConvarsQuery = queryOptions({
    queryKey: ['launch', 'cfg-convars'],
    queryFn: async ({ signal }) => {
        const body = await getJson<unknown>('/servers/default/launch/cfg-convars', signal);
        return listing(body, 'convars') ? (body as CfgConvars) : null;
    },
});

/** FXServer reads convar names without regard to case. */
export const sameName = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();

export const findConvar = (catalog: ConvarCatalog | null | undefined, name: string): Convar | undefined =>
    name.trim() === '' ? undefined : catalog?.convars.find((convar) => sameName(convar.name, name));

/** The cfg's last setting of `name`: FXServer runs the cfg after the command line, so it applies. */
export const cfgSetting = (cfg: CfgConvars | null | undefined, name: string): CfgConvar | undefined =>
    cfg?.convars.findLast((set) => sameName(set.name, name));

/** The values to suggest for a convar: its type's own, or true and false. */
export const suggestedValues = (convar: Convar): string[] =>
    convar.kind === 'bool' ? ['true', 'false'] : convar.values.map(({ value }) => value);

/** Each name the cfg sets, once, with the setting that applies, in the order of those settings. */
export const cfgLastSettings = (cfg: CfgConvars | null | undefined): CfgConvar[] => {
    const last = new Map<string, CfgConvar>();
    for (const set of cfg?.convars ?? []) {
        const key = set.name.toLowerCase();
        if (last.has(key)) {
            last.delete(key);
        }
        last.set(key, set);
    }
    return [...last.values()];
};
