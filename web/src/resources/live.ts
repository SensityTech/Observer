import { useMutation } from '@tanstack/react-query';
import { useState } from 'react';

import type { ResourcesView } from '../api/generated/ResourcesView';
import { orderResource, rescanResources, type ResourceVerb } from '../api/resources';
import { useTopic } from '../live/context';
import { answer, drop, NOTHING_SENT, RESCAN, seen, send, type Sent } from './sent';

/** What the page asks: FXServer's own command on a resource, or a rescan, which names none. */
type Asked = { verb: ResourceVerb; name: string } | { verb: 'rescan' };
type Order = { verb: ResourceVerb; name: string; dependants: boolean } | { verb: 'rescan' };

/**
 * An order that did not simply go through. FXServer was not reading its console: the line waits
 * for it (`pending`), or was never written (`busy`). Or the API refused the order (`failed`).
 */
export type Outcome = Asked & ({ kind: 'pending' } | { kind: 'busy' } | { kind: 'failed'; error: unknown });

const keyOf = (order: Order) => (order.verb === 'rescan' ? RESCAN : order.name);
const askedBy = (order: Order): Asked =>
    order.verb === 'rescan' ? { verb: 'rescan' } : { verb: order.verb, name: order.name };

/**
 * The resources of the run that is up as `server.resources` says them, whole at each change, with
 * the orders of this page. `view` is null until the topic's snapshot, while the link is down and
 * for an account the topic refuses.
 */
export const useResources = () => {
    const [view, setView] = useState<ResourcesView | null>(null);
    const [sent, setSent] = useState<Sent>(NOTHING_SENT);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const took = (next: ResourcesView) => {
        setView(next);
        setSent(seen);
    };
    const lost = () => {
        setView(null);
        setSent(NOTHING_SENT);
    };
    useTopic('server.resources', { snapshot: took, event: took, refused: lost, cleared: lost });

    const { mutate } = useMutation({
        mutationFn: (order: Order) =>
            order.verb === 'rescan'
                ? rescanResources()
                : orderResource(order.verb, { name: order.name, dependants: order.dependants }),
        onMutate: (order) => {
            setOutcome(null);
            setSent((held) => send(held, keyOf(order), order.verb));
        },
        onSuccess: ({ delivery }, order) => {
            setSent((held) => (delivery === 'busy' ? drop(held, keyOf(order)) : answer(held, keyOf(order))));
            if (delivery !== 'written') {
                setOutcome({ ...askedBy(order), kind: delivery });
            }
        },
        onError: (error, order) => {
            setSent((held) => drop(held, keyOf(order)));
            setOutcome({ ...askedBy(order), kind: 'failed', error });
        },
    });
    // One order at a time for a resource, as the API takes them.
    const ask = (order: Order) => {
        if (!sent.has(keyOf(order))) {
            mutate(order);
        }
    };

    return {
        view,
        sent,
        outcome,
        dismiss: () => setOutcome(null),
        /** `dependants`, for a restart: start again the resources FXServer stops with this one. */
        order: (verb: ResourceVerb, name: string, dependants = false) => ask({ verb, name, dependants }),
        rescan: () => ask({ verb: 'rescan' }),
    };
};
