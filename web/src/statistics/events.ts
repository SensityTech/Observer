import type { EventLink } from '../api/generated/EventLink';
import type { TimelineEvent } from '../api/generated/TimelineEvent';
import type { Messages } from '../i18n/en';

/** How far around the cursor the panel looks for events: a quarter hour, or a 48th of the window. */
export const AROUND = 15 * 60;
const AROUND_SHARE = 1 / 48;

export type EventKind = TimelineEvent['detail']['kind'];

/** Drawn in danger: what took players away. */
export const isTrouble = (event: TimelineEvent) =>
    event.detail.kind === 'crash' || event.detail.kind === 'crashWave' || event.detail.kind === 'incident';

/** An event in words, as the strip, the panel and the list say it. */
export const eventWords = (
    t: Messages,
    event: TimelineEvent,
    words: {
        duration: (seconds: number) => string;
        clock: (seconds: number) => string;
        ms: (ms: number) => string;
    },
) => {
    const e = t.statistics.events;
    const detail = event.detail;
    switch (detail.kind) {
        case 'start':
            return detail.build === null ? e.start : `${e.start}, ${e.build(detail.build)}`;
        case 'restart':
            return detail.build === null ? e.restart : `${e.restart}, ${e.build(detail.build)}`;
        case 'stop':
            return e.stop;
        case 'crash': {
            const what = detail.hang ? e.hang : e.crash;
            return detail.backOnlineAt === null
                ? what
                : `${what}, ${e.backOnline(words.clock(detail.backOnlineAt))}`;
        }
        case 'changes': {
            const named = detail.resources;
            const others = Math.max(
                0,
                detail.counts.added +
                    detail.counts.removed +
                    detail.counts.updated +
                    detail.counts.sameVersion +
                    detail.counts.renamed +
                    detail.counts.moved -
                    named.length,
            );
            if (named.length > 0) {
                return e.changes(named, others);
            }
            return detail.build !== null ? e.buildChange(detail.build.to) : e.changes([], 0);
        }
        case 'resource':
            return e.resource[detail.change](detail.name);
        case 'files': {
            const files = e.files(detail.files, detail.resources);
            return detail.by.length > 0 ? `${files} ${e.by(detail.by)}` : files;
        }
        case 'crashWave':
            return e.wave(
                detail.crashes,
                detail.players,
                words.duration((event.until ?? event.at) - event.at),
            );
        case 'incident':
            return detail.hitchMs === null
                ? e.incident(detail.drops)
                : `${e.incident(detail.drops)}${e.duringHitch(words.ms(detail.hitchMs))}`;
        case 'backup':
            return detail.failed ? e.backupFailed : e.backup;
    }
};

/** The short label the strip shows beside an event's icon. */
export const eventLabel = (t: Messages, event: TimelineEvent, words: Parameters<typeof eventWords>[2]) => {
    const e = t.statistics.events;
    switch (event.detail.kind) {
        case 'crashWave':
            return e.waveShort(event.detail.crashes);
        case 'restart':
            return e.restart;
        case 'start':
            return e.start;
        case 'crash':
            return event.detail.hang ? e.hang : e.crash;
        default:
            return eventWords(t, event, words);
    }
};

/** The events near `time` (Unix seconds) in a window of `span` seconds, oldest first. */
export const eventsAround = (events: readonly TimelineEvent[], time: number, span: number) => {
    const reach = Math.max(AROUND, span * AROUND_SHARE);
    return events.filter((event) => event.at - reach <= time && (event.until ?? event.at) + reach >= time);
};

/** The links whose change or wave falls in the window, newest wave first. */
export const linksIn = (links: readonly EventLink[], events: readonly TimelineEvent[]) => {
    const byKey = new Map(events.map((event) => [event.key, event]));
    return links
        .filter((link) => byKey.has(link.wave) || byKey.has(link.change))
        .toSorted((a, b) => (byKey.get(b.wave)?.at ?? 0) - (byKey.get(a.wave)?.at ?? 0));
};

/** The start a change event is, for the Changes page: `start:12` reads 12. */
export const startOf = (key: string) => {
    const match = /^start:(\d+)$/.exec(key);
    return match === null ? null : Number(match[1]);
};
