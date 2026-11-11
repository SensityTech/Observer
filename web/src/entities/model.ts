import type { EntitiesLive } from '../api/generated/EntitiesLive';
import type { EntityItem } from '../api/generated/EntityItem';
import type { EntityKind } from '../api/generated/EntityKind';
import type { EntityOutcome } from '../api/generated/EntityOutcome';
import type { EntityResult } from '../api/generated/EntityResult';

/** `GetEntityPopulationType` for what a script created. */
const POPULATION_SCRIPT = 7;
/** The groups a deletion's dialog names, past which the rest is only counted. */
const MOST_GROUPS = 5;

/** A model's hash as the game's tools write it: `0xB779A091`. The server knows no other name for it. */
export const modelHash = (model: number) => `0x${(model >>> 0).toString(16).toUpperCase().padStart(8, '0')}`;

/** A net id and the serial the bridge drew for the entity: net ids are used again. */
export const keyOf = (item: Pick<EntityItem, 'netId' | 'serial'>) => `${item.netId}:${item.serial}`;

/** Why the panel never offers to delete an entity, or null when it does. */
export const blocker = (item: Pick<EntityItem, 'occupied' | 'class'>): 'occupied' | 'train' | null =>
    item.occupied ? 'occupied' : item.class === 'train' ? 'train' : null;

/** Whether a script created the entity, a resource of the server's or a client's. */
export const scripted = (item: Pick<EntityItem, 'resource' | 'population'>) =>
    item.resource !== null || item.population === POPULATION_SCRIPT;

/**
 * Whether the list is too old to delete from: the server stopped sweeping. The same rule as the
 * server's, on its own cadence, so that the panel does not offer what the API would refuse.
 */
export const isStale = (live: Pick<EntitiesLive, 'at' | 'everyMs'>, now: number, sweeps: number) =>
    live.at !== null && now - Date.parse(live.at) > sweeps * Math.max(live.everyMs, 1000);

export type Group = { model: number; kind: EntityKind; resource: string | null; count: number };

/** What a selection is made of: its largest groups, and what a deletion should warn about. */
export const breakdown = (items: readonly EntityItem[]) => {
    const groups = new Map<string, Group>();
    for (const item of items) {
        const key = `${item.model}:${item.kind}:${item.resource ?? ''}`;
        const group = groups.get(key);
        if (group === undefined) {
            groups.set(key, { model: item.model, kind: item.kind, resource: item.resource, count: 1 });
        } else {
            group.count += 1;
        }
    }
    const sorted = [...groups.values()].toSorted(
        (left, right) => right.count - left.count || left.model - right.model,
    );
    return {
        groups: sorted.slice(0, MOST_GROUPS),
        more: sorted.slice(MOST_GROUPS).reduce((sum, group) => sum + group.count, 0),
        scripted: items.filter(scripted).length,
        owned: items.filter((item) => item.owned).length,
    };
};

/** What became of the entities of a deletion: how many were deleted, and the others by outcome. */
export const outcomes = (results: readonly EntityResult[]) => {
    const others = new Map<Exclude<EntityOutcome, 'deleted'>, number>();
    let deleted = 0;
    for (const { outcome } of results) {
        if (outcome === 'deleted') {
            deleted += 1;
        } else {
            others.set(outcome, (others.get(outcome) ?? 0) + 1);
        }
    }
    return { asked: results.length, deleted, others: [...others] };
};
