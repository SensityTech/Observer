import type { ActorKind } from '../api/generated/ActorKind';
import type { AuditAction } from '../api/generated/AuditAction';
import type { AuditEntry } from '../api/generated/AuditEntry';
import type { Outcome } from '../api/generated/Outcome';
import type { ServerMessage } from '../api/generated/ServerMessage';
import type { Topic } from '../api/generated/Topic';

// What arrives on the socket is untrusted JSON until it matches the contract field by field. The
// records below list every value of the generated unions: one the Rust side adds fails the type check
// here until it is listed.

const TOPICS = { status: true, audit: true } satisfies Record<Topic, true>;

const ACTOR_KINDS = {
    user: true,
    local: true,
    cli: true,
    system: true,
    anonymous: true,
} satisfies Record<ActorKind, true>;

const AUDIT_ACTIONS = {
    setup_completed: true,
    setup_link_created: true,
    login_succeeded: true,
    login_failed: true,
    logout: true,
    session_revoked: true,
    password_changed: true,
    user_created: true,
    user_deleted: true,
    user_permissions_changed: true,
    user_password_reset: true,
    user_unlocked: true,
    permission_denied: true,
    audit_exported: true,
    reauthenticated: true,
    launch_settings_changed: true,
} satisfies Record<AuditAction, true>;

const OUTCOMES = { success: true, failure: true, denied: true } satisfies Record<Outcome, true>;

type Json = Record<string, unknown>;

const isRecord = (value: unknown): value is Json =>
    typeof value === 'object' && value !== null && !Array.isArray(value);

const isId = (value: unknown): value is number => Number.isSafeInteger(value);

const isText = (value: unknown): value is string => typeof value === 'string';

const isTimestamp = (value: unknown): value is string => isText(value) && !Number.isNaN(Date.parse(value));

const orNull =
    <T>(check: (value: unknown) => value is T) =>
    (value: unknown): value is T | null =>
        value === null || check(value);

const oneOf =
    <T extends string>(values: Record<T, true>) =>
    (value: unknown): value is T =>
        isText(value) && Object.hasOwn(values, value);

const isTopic = oneOf<Topic>(TOPICS);

export const isAuditEntry = (value: unknown): value is AuditEntry =>
    isRecord(value) &&
    isId(value.id) &&
    isTimestamp(value.at) &&
    oneOf<ActorKind>(ACTOR_KINDS)(value.actorKind) &&
    orNull(isId)(value.actorId) &&
    orNull(isText)(value.actorName) &&
    orNull(isText)(value.ip) &&
    orNull(isText)(value.userAgent) &&
    oneOf<AuditAction>(AUDIT_ACTIONS)(value.action) &&
    orNull(isText)(value.target) &&
    oneOf<Outcome>(OUTCOMES)(value.outcome) &&
    isRecord(value.details);

const isSnapshotData = (topic: Topic, data: unknown) => {
    if (!isRecord(data)) {
        return false;
    }
    switch (topic) {
        case 'status':
            return isText(data.version) && isTimestamp(data.startedAt);
        case 'audit':
            return orNull(isId)(data.latestId);
    }
};

const isEventData = (topic: Topic, data: unknown) => {
    switch (topic) {
        case 'status':
            return false;
        case 'audit':
            return isAuditEntry(data);
    }
};

/** A message from Observer, or null when it is not one the contract knows. */
export const parseServerMessage = (text: string): ServerMessage | null => {
    let message: unknown;
    try {
        message = JSON.parse(text);
    } catch {
        return null;
    }
    if (!isRecord(message)) {
        return null;
    }
    const valid = (() => {
        switch (message.type) {
            case 'hello':
                return isId(message.connectionId) && isId(message.contract);
            case 'snapshot':
                return isTopic(message.topic) && isSnapshotData(message.topic, message.data);
            case 'event':
                return isTopic(message.topic) && isEventData(message.topic, message.data);
            case 'resync':
                return isTopic(message.topic);
            case 'unsubscribed':
                return isTopic(message.topic) && message.reason === 'forbidden';
            case 'pong':
                return true;
            // The panel does not act on these; a code it does not know reads as any other.
            case 'error':
                return isText(message.code);
            default:
                return false;
        }
    })();
    return valid ? (message as ServerMessage) : null;
};
