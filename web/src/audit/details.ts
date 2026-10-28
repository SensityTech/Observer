import { parsePermissions, permissionKeys } from '../auth/permissions';
import type { Messages } from '../i18n/en';

type Format = {
    number: (value: number) => string;
    timestamp: (rfc3339: string) => string;
};

const IDS = new Set<string>(['userId', 'sessionId']);
const MASKS = new Set<string>(['permissions', 'added', 'removed']);
const TIMES = new Set<string>([
    'firstAt',
    'lastAt',
    'from',
    'to',
    'at',
    'occurrence',
    'previousAt',
    'startedAt',
]);
const ACTIONS = new Set<string>(['attempted', 'action']);
/** The schedule's lists, and the nested entries of its audit details. */
const LISTS = new Set<string>(['times', 'warnings', 'dropped']);
const NESTED = new Set<string>(['scheduled', 'dropped']);
/** The schedule's settings counted in minutes. */
const MINUTES = new Set<string>(['intervalMinutes', 'skipAfterStart', 'warnings']);

const has = <T extends object>(record: T, key: unknown): key is keyof T =>
    typeof key === 'string' && Object.hasOwn(record, key);

/** A change the log records as the value before and the value after, such as a launch setting. */
const isChange = (value: unknown): value is { before: unknown; after: unknown } =>
    typeof value === 'object' &&
    value !== null &&
    !Array.isArray(value) &&
    Object.keys(value).length === 2 &&
    'before' in value &&
    'after' in value;

const isNames = (value: unknown): value is string[] =>
    Array.isArray(value) && value.every((name) => typeof name === 'string');

/** The extra arguments a launch change set or removed, by name only: their values stay out of the log. */
const isArgumentChange = (value: unknown): value is { set: string[]; removed: string[] } =>
    typeof value === 'object' &&
    value !== null &&
    'set' in value &&
    isNames(value.set) &&
    'removed' in value &&
    isNames(value.removed);

const isRecord = (value: unknown): value is Record<string, unknown> =>
    typeof value === 'object' && value !== null && !Array.isArray(value);

/** Launch settings the log only says changed. */
const HIDDEN = new Set<string>(['buildPath', 'dataPath', 'cfgFile']);

/**
 * An entry's details as one line of `label: value` pairs, in the reader's words: permission names
 * rather than masks, local times, catalog names for actions and refusals, `before → after` for a
 * change. A key or a value the catalog does not know reads as the log stores it.
 */
export const auditDetails = (t: Messages, format: Format, details: Record<string, unknown>) =>
    pairs(t, format, details).join(' · ');

const pairs = (t: Messages, format: Format, details: Record<string, unknown>) =>
    Object.entries(details).map(
        ([key, value]) =>
            `${has(t.audit.detail, key) ? t.audit.detail[key] : key}: ${word(t, format, key, value)}`,
    );

const word = (t: Messages, format: Format, key: string, value: unknown): string => {
    if (isChange(value)) {
        return `${word(t, format, key, value.before)} → ${word(t, format, key, value.after)}`;
    }
    if (value === null) {
        return key === 'timeZone' ? t.audit.hostZone : t.audit.unset;
    }
    if (HIDDEN.has(key) && value === true) {
        return t.audit.changed;
    }
    if (typeof value === 'boolean') {
        return value ? t.audit.yes : t.audit.no;
    }
    if (isArgumentChange(value)) {
        const parts = [
            value.set.length > 0 && t.audit.argumentsSet(value.set.join(', ')),
            value.removed.length > 0 && t.audit.argumentsRemoved(value.removed.join(', ')),
        ].filter((part) => part !== false);
        return parts.length > 0 ? parts.join('; ') : t.audit.argumentsReordered;
    }
    if (typeof value === 'number') {
        if (key === 'lateBySeconds') {
            return t.audit.seconds(format.number(value));
        }
        if (MINUTES.has(key)) {
            return t.audit.minutes(format.number(value));
        }
        return IDS.has(key) ? String(value) : format.number(value);
    }
    // A schedule's lists and what a change dropped: their items, one after the other.
    if (LISTS.has(key) && Array.isArray(value)) {
        return value.length === 0
            ? t.audit.unset
            : value.map((item) => word(t, format, key, item)).join(', ');
    }
    if (NESTED.has(key) && isRecord(value)) {
        return `(${pairs(t, format, value).join(', ')})`;
    }
    if (typeof value !== 'string') {
        return JSON.stringify(value);
    }
    if (MASKS.has(key)) {
        const names = permissionKeys(parsePermissions(value)).map(
            (permission) => t.permissions[permission].name,
        );
        return names.length > 0 ? names.join(', ') : t.audit.noPermission;
    }
    // `from` is a time in a filter, and where a stop of Observer came from.
    if (key === 'from' && has(t.audit.stopSources, value)) {
        return t.audit.stopSources[value];
    }
    if (TIMES.has(key)) {
        return format.timestamp(value);
    }
    if (key === 'permission' && has(t.permissions, value)) {
        return t.permissions[value].name;
    }
    if (ACTIONS.has(key) && has(t.audit.actions, value)) {
        return t.audit.actions[value];
    }
    if (key === 'outcome' && has(t.audit.outcomes, value)) {
        return t.audit.outcomes[value];
    }
    if (key === 'refusal' && has(t.audit.refusals, value)) {
        return t.audit.refusals[value];
    }
    if (key === 'reason' && has(t.audit.launchFailures, value)) {
        return t.audit.launchFailures[value];
    }
    if (key === 'reason' && has(t.audit.missReasons, value)) {
        return t.audit.missReasons[value];
    }
    if (key === 'reason' && has(t.audit.dropReasons, value)) {
        return t.audit.dropReasons[value];
    }
    if (key === 'source' && has(t.audit.sources, value)) {
        return t.audit.sources[value];
    }
    if (key === 'kind' && has(t.audit.dropped, value)) {
        return t.audit.dropped[value];
    }
    if (key === 'mode' && has(t.schedule.modes, value)) {
        return t.schedule.modes[value];
    }
    if (key === 'state' && has(t.server_state.states, value)) {
        return t.server_state.states[value];
    }
    return value;
};
