import type { EntityItem } from '../api/generated/EntityItem';
import type { EntityTarget } from '../api/generated/EntityTarget';
import { blocker, keyOf } from './model';

/** The entities picked for a deletion, as the list last showed each, across its pages. */
export type Selection = ReadonlyMap<string, EntityItem>;

export const NOTHING: Selection = new Map();

/** Whether the panel offers to pick the entity. */
export const pickable = (item: EntityItem) => blocker(item) === null;

/** The selection with `item` picked or let go. Picking past `most`, or what is never deleted, changes nothing. */
export const toggled = (selection: Selection, item: EntityItem, most: number): Selection => {
    const key = keyOf(item);
    if (selection.has(key)) {
        const next = new Map(selection);
        next.delete(key);
        return next;
    }
    return pickable(item) && selection.size < most ? new Map(selection).set(key, item) : selection;
};

/** How much of what a page offers is picked. */
export const pageState = (selection: Selection, items: readonly EntityItem[]): 'all' | 'some' | 'none' => {
    const offered = items.filter(pickable);
    const picked = offered.filter((item) => selection.has(keyOf(item))).length;
    if (picked === 0) {
        return 'none';
    }
    return picked === offered.length ? 'all' : 'some';
};

/** The selection with a whole page let go when it all was picked, picked as far as `most` goes otherwise. */
export const pageToggled = (selection: Selection, items: readonly EntityItem[], most: number): Selection => {
    const next = new Map(selection);
    if (pageState(selection, items) === 'all') {
        for (const item of items) {
            next.delete(keyOf(item));
        }
        return next;
    }
    for (const item of items.filter(pickable)) {
        if (next.size >= most) {
            break;
        }
        next.set(keyOf(item), item);
    }
    return next;
};

/**
 * The selection once the list answered again: an entity seen again is kept as it now is, and let
 * go once a player sits in it. The same selection is given back when nothing of it changed.
 */
export const refreshed = (selection: Selection, items: readonly EntityItem[]): Selection => {
    let next: Map<string, EntityItem> | null = null;
    for (const item of items) {
        const key = keyOf(item);
        const picked = selection.get(key);
        if (picked === undefined || JSON.stringify(picked) === JSON.stringify(item)) {
            continue;
        }
        next ??= new Map(selection);
        if (pickable(item)) {
            next.set(key, item);
        } else {
            next.delete(key);
        }
    }
    return next ?? selection;
};

/** What a deletion of the selection asks the server for. */
export const targetsOf = (selection: Selection): EntityTarget[] =>
    [...selection.values()].map(({ netId, serial, model }) => ({ netId, serial, model }));
