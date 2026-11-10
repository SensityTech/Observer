import { queryOptions, type Query } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { DebugBags } from './generated/DebugBags';
import type { DebugEvents } from './generated/DebugEvents';
import type { DebugHttp } from './generated/DebugHttp';
import type { DebugWindow } from './generated/DebugWindow';
import type { KnownHost } from './generated/KnownHost';

/** The resources report every second, and Observer totals them in slices of five. */
const EVERY_MS = 2_000;

/** A page asks again while it is live, and stops once its feature answers that it is off. */
const every =
    (live: boolean) =>
    ({ state }: Pick<Query, 'state'>) =>
        live && state.status !== 'error' ? EVERY_MS : false;

export const debugEventsQuery = (window: DebugWindow, live: boolean) =>
    queryOptions({
        queryKey: ['debug', 'events', window],
        queryFn: ({ signal }) => getJson<DebugEvents>(`/debug/events?window=${window}`, signal),
        refetchInterval: every(live),
    });

export const debugBagsQuery = (window: DebugWindow, live: boolean) =>
    queryOptions({
        queryKey: ['debug', 'bags', window],
        queryFn: ({ signal }) => getJson<DebugBags>(`/debug/bags?window=${window}`, signal),
        refetchInterval: every(live),
    });

export const debugHttpQuery = (window: DebugWindow, live: boolean) =>
    queryOptions({
        queryKey: ['debug', 'http', window],
        queryFn: ({ signal }) => getJson<DebugHttp>(`/debug/http?window=${window}`, signal),
        refetchInterval: every(live),
    });

/** The owner knows why this resource calls this host: it stops showing as new. */
export const knowHost = (known: KnownHost) => send('POST', '/debug/http/known', known);
