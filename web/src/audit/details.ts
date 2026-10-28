import { parsePermissions, permissionKeys } from '../auth/permissions';
import type { Messages } from '../i18n/en';

type Format = {
    number: (value: number) => string;
    timestamp: (rfc3339: string) => string;
};

const IDS = new Set<string>(['userId', 'sessionId']);
const MASKS = new Set<string>(['permissions', 'added', 'removed']);
const TIMES = new Set<string>(['firstAt', 'lastAt', 'from', 'to']);
const ACTIONS = new Set<string>(['attempted', 'action']);

const has = <T extends object>(record: T, key: unknown): key is keyof T =>
    typeof key === 'string' && Object.hasOwn(record, key);

/**
 * An entry's details as one line of `label: value` pairs, in the reader's words: permission names
 * rather than masks, local times, catalog names for actions and refusals. A key or a value the
 * catalog does not know reads as the log stores it.
 */
export const auditDetails = (t: Messages, format: Format, details: Record<string, unknown>) =>
    Object.entries(details)
        .map(
            ([key, value]) =>
                `${has(t.audit.detail, key) ? t.audit.detail[key] : key}: ${word(t, format, key, value)}`,
        )
        .join(' · ');

const word = (t: Messages, format: Format, key: string, value: unknown): string => {
    if (typeof value === 'number') {
        return IDS.has(key) ? String(value) : format.number(value);
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
    return value;
};
