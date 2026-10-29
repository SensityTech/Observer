import type { Crash } from '../api/generated/Crash';
import type { DiagnosticsEvent } from '../api/generated/DiagnosticsEvent';
import type { DiagnosticsSnapshot } from '../api/generated/DiagnosticsSnapshot';
import type { TreeProcess } from '../api/generated/TreeProcess';

/** What the page keeps, as the snapshot holds it. */
export const MAX_PROCESSES = 200;
export const MAX_CRASHES = 50;

/** Puts `item` in place of the one with its id, or adds it, then keeps the newest `max`. */
const upsert = <T extends { id: number }>(items: T[], item: T, newer: (a: T, b: T) => number, max: number) =>
    [item, ...items.filter((each) => each.id !== item.id)].toSorted(newer).slice(0, max);

const latestStart = (a: TreeProcess, b: TreeProcess) =>
    Date.parse(b.lastStartedAt) - Date.parse(a.lastStartedAt) || b.id - a.id;

const latestCrash = (a: Crash, b: Crash) => Date.parse(b.at) - Date.parse(a.at) || b.id - a.id;

/** The snapshot once `event` is applied to it. */
export const applyEvent = (snapshot: DiagnosticsSnapshot, event: DiagnosticsEvent): DiagnosticsSnapshot => {
    switch (event.kind) {
        case 'process':
            return {
                ...snapshot,
                processes: upsert(snapshot.processes, event.process, latestStart, MAX_PROCESSES),
            };
        case 'connections':
            return { ...snapshot, destinations: event.destinations, listeners: event.listeners };
        case 'crash':
            return { ...snapshot, crashes: upsert(snapshot.crashes, event.crash, latestCrash, MAX_CRASHES) };
        case 'crashFolder':
            return { ...snapshot, crashFolder: event.folder };
    }
};
