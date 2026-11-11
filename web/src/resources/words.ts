import type { ResourceRow } from '../api/generated/ResourceRow';
import type { ResourceWarnings } from '../api/generated/ResourceWarnings';
import type { ResourceVerb } from '../api/resources';
import type { Messages } from '../i18n/en';
import { secondsSince } from '../lib/useNow';
import { didNotStart } from './list';

type Format = { duration: (seconds: number) => string };

/** An order FXServer has not answered yet, and who asked for it. */
export type Order = { verb: ResourceVerb; by: string };

/** How long ago, to the minute: under one, it happened just now. */
const ago = (t: Messages, format: Format, at: string, now: number) => {
    const seconds = secondsSince(at, now);
    return t.resources.ago(seconds < 60 ? null : format.duration(seconds));
};

/**
 * What a row says under its state: the order under way and who asked for it, or else that FXServer
 * could not start the resource, or else what last happened to it in this run.
 */
export const changeLine = (
    t: Messages,
    format: Format,
    row: ResourceRow,
    now: number,
    order: Order | null,
) => {
    const words = t.resources;
    if (order !== null) {
        return words.asked[order.verb](order.by);
    }
    if (row.failedAt !== null && didNotStart(row)) {
        return words.couldNotStart(ago(t, format, row.failedAt, now));
    }
    if (row.last === null) {
        return null;
    }
    const { kind, by, with: along, at } = row.last;
    return words.happened(words.last[kind], by, along, ago(t, format, at, now));
};

/**
 * What FXServer warned of as it started a resource, a line each: the streamed files it finds
 * heavy, the files the manifest names without holding them, and what it counted besides.
 */
export const warningLines = (t: Messages, warnings: ResourceWarnings) => {
    const words = t.resources;
    const others = warnings.count - warnings.heavy - warnings.missing;
    return [
        warnings.heavy > 0 && words.heavy(warnings.heavy, warnings.oversized),
        warnings.missing > 0 && words.missing(warnings.missing),
        others > 0 && words.otherWarnings(others),
    ].filter((line) => line !== false);
};
