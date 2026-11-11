import type { AuditAction } from '../api/generated/AuditAction';
import { parsePermissions, permissionKeys } from '../auth/permissions';
import { GB, MB, MIB } from '../changes/units';
import type { Messages } from '../i18n/en';

type Format = {
    number: (value: number) => string;
    bytes: (bytes: number) => string;
    timestamp: (rfc3339: string) => string;
};

const IDS = new Set<string>(['userId', 'sessionId', 'pid', 'crash', 'serverId']);
/** FXServer build numbers, which read as they are written, not as quantities. */
const BUILDS = new Set<string>(['build', 'from', 'to', 'previous']);
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
    'until',
    'since',
]);
const ACTIONS = new Set<string>(['attempted', 'action']);
/** The schedule's lists, the resources an order stopped, and the nested entries of the details. */
const LISTS = new Set<string>([
    'times',
    'warnings',
    'dropped',
    'branches',
    'extensions',
    'excluded',
    'hidden',
    'dependants',
]);
const NESTED = new Set<string>(['scheduled', 'dropped']);
/** The schedule's settings counted in minutes. */
const MINUTES = new Set<string>(['intervalMinutes', 'skipAfterStart', 'warnings']);
/** The players' retentions and change tracking's, in days. */
const DAYS = new Set<string>([
    'linkDays',
    'addressDays',
    'dayDays',
    'profileDays',
    'retentionDays',
    'verifyDays',
]);

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

/** Entities a deletion took together: one model, one kind, one creating resource. */
const isEntityGroup = (
    value: unknown,
): value is { model: string; kind: string; resource: string | null; count: number } =>
    isRecord(value) &&
    typeof value.model === 'string' &&
    typeof value.kind === 'string' &&
    (value.resource === null || typeof value.resource === 'string') &&
    typeof value.count === 'number';

/** Settings the log records whole, before and after: change tracking's. */
const isWholeChange = (
    details: Record<string, unknown>,
    action: AuditAction | undefined,
): details is { before: Record<string, unknown>; after: Record<string, unknown> } =>
    action === 'change_settings_changed' &&
    isChange(details) &&
    isRecord(details.before) &&
    isRecord(details.after);

/** A whole change as the settings it changed, each before and after. */
const changedSettings = ({
    before,
    after,
}: {
    before: Record<string, unknown>;
    after: Record<string, unknown>;
}) =>
    Object.fromEntries(
        Object.keys({ ...before, ...after })
            .filter((key) => JSON.stringify(before[key]) !== JSON.stringify(after[key]))
            .map((key) => [key, { before: before[key], after: after[key] }]),
    );

/** Launch settings the log only says changed. */
const HIDDEN = new Set<string>(['buildPath', 'dataPath', 'cfgFile']);

/**
 * An entry's details as one line of `label: value` pairs, in the reader's words: permission names
 * rather than masks, local times, catalog names for actions and refusals, `before → after` for a
 * change. A key or a value the catalog does not know reads as the log stores it.
 */
export const auditDetails = (
    t: Messages,
    format: Format,
    details: Record<string, unknown>,
    action?: AuditAction,
) =>
    pairs(
        t,
        format,
        isWholeChange(details, action) ? changedSettings(details) : details,
        action === 'change_contents_viewed',
    ).join(' · ');

const pairs = (t: Messages, format: Format, details: Record<string, unknown>, snapshots = false) =>
    Object.entries(details).map(
        ([key, value]) => `${label(t, key, value, snapshots)}: ${word(t, format, key, value)}`,
    );

/**
 * `from` and `to` name times in most entries, builds in a return to the last healthy one, and in a
 * read of changed contents the two snapshots compared: starts, or a start and a check of the disk.
 */
const label = (t: Messages, key: string, value: unknown, snapshots: boolean) => {
    if (typeof value === 'number' && (key === 'from' || key === 'to')) {
        if (snapshots) {
            return key === 'from' ? t.audit.detail.fromSnapshot : t.audit.detail.toSnapshot;
        }
        return key === 'from' ? t.audit.detail.fromBuild : t.audit.detail.toBuild;
    }
    // The players' retentions, as they were and as they became.
    if (isRecord(value) && (key === 'from' || key === 'to')) {
        return key === 'from' ? t.audit.detail.before : t.audit.detail.after;
    }
    return has(t.audit.detail, key) ? t.audit.detail[key] : key;
};

const word = (t: Messages, format: Format, key: string, value: unknown): string => {
    if (isChange(value)) {
        return `${word(t, format, key, value.before)} → ${word(t, format, key, value.after)}`;
    }
    if (value === null) {
        if (key === 'profileDays') {
            return t.audit.keptForGood;
        }
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
    // A process's parent: its program and pid, as the console names it.
    if (key === 'parent' && isRecord(value)) {
        return [value.name, value.pid].filter((part) => part !== null && part !== undefined).join(' ');
    }
    if (typeof value === 'number') {
        if (key === 'tookMs') {
            return t.audit.milliseconds(format.number(value));
        }
        if (key === 'generation') {
            return t.diagnostics.processes.generation(value);
        }
        if (key === 'lateBySeconds' || key === 'everySeconds') {
            return t.audit.seconds(format.number(value));
        }
        // Kept in seconds, in the minutes the launch settings take it in.
        if (key === 'hangTimeoutSeconds') {
            return t.audit.minutes(format.number(value / 60));
        }
        if (MINUTES.has(key)) {
            return t.audit.minutes(format.number(value));
        }
        if (DAYS.has(key)) {
            return t.audit.days(value);
        }
        if (key === 'bytes') {
            return format.bytes(value);
        }
        // Change tracking's settings, in the units its form takes them in.
        if (key === 'maxText') {
            return t.audit.megabytes(format.number(value / MIB));
        }
        if (key === 'budget') {
            return t.audit.gigabytes(format.number(value / GB));
        }
        if (key === 'hashRate') {
            return t.audit.rate(t.audit.megabytes(format.number(value / MB)));
        }
        return IDS.has(key) || BUILDS.has(key) ? String(value) : format.number(value);
    }
    // A schedule's lists and what a change dropped: their items, one after the other.
    if ((LISTS.has(key) || key === 'names') && Array.isArray(value)) {
        return value.length === 0
            ? t.audit.unset
            : value.map((item) => word(t, format, key, item)).join(', ');
    }
    if ((NESTED.has(key) || key === 'from' || key === 'to') && isRecord(value)) {
        return `(${pairs(t, format, value).join(', ')})`;
    }
    // What a deletion of entities left, by what became of each, then what it took.
    if (key === 'outcomes' && isRecord(value)) {
        return Object.entries(value)
            .map(([outcome, left]) =>
                has(t.entities.notDeleted, outcome) && typeof left === 'number'
                    ? t.entities.notDeleted[outcome](left)
                    : `${outcome}: ${JSON.stringify(left)}`,
            )
            .join(', ');
    }
    if (key === 'entities' && Array.isArray(value)) {
        return value
            .map((group) =>
                isEntityGroup(group)
                    ? t.entities.group(
                          group.count,
                          group.model,
                          has(t.entities.kinds, group.kind) ? t.entities.kinds[group.kind] : group.kind,
                          group.resource,
                      )
                    : JSON.stringify(group),
            )
            .join(', ');
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
    if (key === 'kind' && has(t.audit.buildKinds, value)) {
        return t.audit.buildKinds[value];
    }
    if (key === 'step' && has(t.artifacts.job.steps, value)) {
        return t.artifacts.job.steps[value];
    }
    if (key === 'apply' && has(t.audit.applyModes, value)) {
        return t.audit.applyModes[value];
    }
    if ((key === 'reason' || key === 'why') && has(t.audit.buildReasons, value)) {
        return t.audit.buildReasons[value];
    }
    if (key === 'kind' && has(t.diagnostics.processes.kinds, value)) {
        return t.diagnostics.processes.kinds[value];
    }
    if (key === 'kind' && has(t.audit.snapshotKinds, value)) {
        return t.audit.snapshotKinds[value];
    }
    if (key === 'reason' && has(t.audit.snapshotFailures, value)) {
        return t.audit.snapshotFailures[value];
    }
    if (key === 'view' && has(t.audit.views, value)) {
        return t.audit.views[value];
    }
    if (key === 'mode' && has(t.schedule.modes, value)) {
        return t.schedule.modes[value];
    }
    if (key === 'state' && has(t.server_state.states, value)) {
        return t.server_state.states[value];
    }
    return value;
};
