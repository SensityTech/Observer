import type { LiveChangeKind } from '../api/generated/LiveChangeKind';
import type { ResourceRow } from '../api/generated/ResourceRow';
import type { ResourceVerb } from '../api/resources';

type Order = { verb: ResourceVerb | 'rescan'; answered: boolean };

/**
 * The orders this page sent that the list does not show yet, by resource. The list says an order
 * waits on FXServer from the view that follows it, and that view may come before the API's answer
 * or after it: an order is held from the click until the first view that comes after the answer.
 */
export type Sent = ReadonlyMap<string, Order>;

export const NOTHING_SENT: Sent = new Map();

/** What a rescan is held under: no resource has an empty name. */
export const RESCAN = '';

export const send = (held: Sent, name: string, verb: Order['verb']): Sent =>
    new Map(held).set(name, { verb, answered: false });

/** The API took the order: the list shows it from its next view on. */
export const answer = (held: Sent, name: string): Sent => {
    const order = held.get(name);
    return order === undefined ? held : new Map(held).set(name, { ...order, answered: true });
};

/** The order went nowhere: refused, or never written to FXServer's console. */
export const drop = (held: Sent, name: string): Sent => {
    if (!held.has(name)) {
        return held;
    }
    const left = new Map(held);
    left.delete(name);
    return left;
};

/** A view of the list came: the orders answered before it are the list's to show. */
export const seen = (held: Sent): Sent => {
    const left = new Map([...held].filter(([, order]) => !order.answered));
    return left.size === held.size ? held : left;
};

const VERBS = { started: 'start', stopped: 'stop', restarted: 'restart' } satisfies Record<
    LiveChangeKind,
    ResourceVerb
>;

/**
 * The order under way on a resource: one this page just sent, or one of anyone's the list says
 * FXServer has not answered. No other is taken meanwhile.
 */
export const underWay = (row: ResourceRow, held: Sent): ResourceVerb | null => {
    const mine = held.get(row.name)?.verb;
    if (mine !== undefined && mine !== 'rescan') {
        return mine;
    }
    return row.asked === null ? null : VERBS[row.asked.kind];
};
