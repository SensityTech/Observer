import type { ActorKind } from '../api/generated/ActorKind';
import type { AuditAction } from '../api/generated/AuditAction';
import type { AuditEntry } from '../api/generated/AuditEntry';
import type { ConsoleLine } from '../api/generated/ConsoleLine';
import type { HangCause } from '../api/generated/HangCause';
import type { HealthProblem } from '../api/generated/HealthProblem';
import type { LaunchReadiness } from '../api/generated/LaunchReadiness';
import type { LineKind } from '../api/generated/LineKind';
import type { Outcome } from '../api/generated/Outcome';
import type { ServerMessage } from '../api/generated/ServerMessage';
import type { ServerState } from '../api/generated/ServerState';
import type { ServerStatus } from '../api/generated/ServerStatus';
import type { SpawnFailure } from '../api/generated/SpawnFailure';
import type { StateCause } from '../api/generated/StateCause';
import type { Topic } from '../api/generated/Topic';

// What arrives on the socket is untrusted JSON until it matches the contract field by field. The
// records below list every value of the generated unions: one the Rust side adds fails the type check
// here until it is listed.

const TOPICS = { status: true, audit: true, 'server.status': true, console: true } satisfies Record<
    Topic,
    true
>;

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
    server_started: true,
    server_stopped: true,
    server_restarted: true,
    console_command_sent: true,
    console_log_downloaded: true,
    server_crash_loop: true,
    schedule_changed: true,
    scheduled_restart_skipped: true,
    scheduled_restart_restored: true,
    scheduled_restart_delayed: true,
    scheduled_restart_missed: true,
    one_off_restart_planned: true,
    one_off_restart_cancelled: true,
} satisfies Record<AuditAction, true>;

const OUTCOMES = { success: true, failure: true, denied: true } satisfies Record<Outcome, true>;

const SERVER_STATES = {
    stopped: true,
    starting: true,
    online: true,
    degraded: true,
    hung: true,
    stopping: true,
    crashed: true,
    crashLoop: true,
} satisfies Record<ServerState, true>;

const PROBLEMS = {
    httpLate: true,
    heartbeatLate: true,
    mainThreadStalled: true,
    probeUnauthorized: true,
    watchdogBark: true,
} satisfies Record<HealthProblem, true>;

const HANG_CAUSES = { noHeartbeat: true, noHttp: true } satisfies Record<HangCause, true>;

const SPAWN_FAILURES = {
    notConfigured: true,
    invalidSettings: true,
    notABuild: true,
    missingData: true,
    cfgUnreadable: true,
    bridge: true,
    spawn: true,
} satisfies Record<SpawnFailure, true>;

const READINESS = { ready: true, notConfigured: true, invalid: true } satisfies Record<LaunchReadiness, true>;

const LINE_KINDS = { stdout: true, stderr: true, command: true, observer: true } satisfies Record<
    LineKind,
    true
>;

type Shape = 'text' | 'count' | 'problems' | 'hang' | 'spawnFailure';

/** What each kind of cause carries besides its kind. */
const CAUSE_FIELDS = {
    requested: { by: 'text' },
    restart: { by: 'text' },
    recovery: {},
    ready: {},
    recovered: {},
    unhealthy: { problems: 'problems' },
    hang: { cause: 'hang' },
    exited: { description: 'text' },
    fatalError: { message: 'text' },
    bindError: { address: 'text' },
    licenceRejected: { message: 'text' },
    consoleQuit: { by: 'text' },
    shutdown: {},
    crashLoop: { failures: 'count' },
    spawnFailed: { reason: 'spawnFailure' },
} satisfies Record<StateCause['kind'], Record<string, Shape>>;

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

const isCount = (value: unknown): value is number => isId(value) && value >= 0;

const isProblems = (value: unknown): value is HealthProblem[] =>
    Array.isArray(value) && value.every(oneOf<HealthProblem>(PROBLEMS));

const isCauseKind = (value: unknown): value is StateCause['kind'] =>
    isText(value) && Object.hasOwn(CAUSE_FIELDS, value);

const isCause = (value: unknown): value is StateCause => {
    if (!isRecord(value) || !isCauseKind(value.kind)) {
        return false;
    }
    const fields: Record<string, Shape> = CAUSE_FIELDS[value.kind];
    return Object.entries(fields).every(([name, shape]) => {
        const field = value[name];
        switch (shape) {
            case 'text':
                return isText(field);
            case 'count':
                return isCount(field);
            case 'problems':
                return isProblems(field);
            case 'hang':
                return oneOf<HangCause>(HANG_CAUSES)(field);
            case 'spawnFailure':
                return oneOf<SpawnFailure>(SPAWN_FAILURES)(field);
        }
    });
};

const isPlayers = (value: unknown) => isRecord(value) && isCount(value.online) && orNull(isCount)(value.max);

export const isServerStatus = (value: unknown): value is ServerStatus =>
    isRecord(value) &&
    oneOf<ServerState>(SERVER_STATES)(value.state) &&
    orNull(isCause)(value.cause) &&
    isTimestamp(value.since) &&
    orNull(isTimestamp)(value.startedAt) &&
    isCount(value.generation) &&
    isProblems(value.problems) &&
    orNull(isCount)(value.build) &&
    (value.players === null || isPlayers(value.players)) &&
    orNull(isTimestamp)(value.restartAt) &&
    oneOf<LaunchReadiness>(READINESS)(value.launch);

export const isConsoleLine = (value: unknown): value is ConsoleLine =>
    Array.isArray(value) &&
    value.length === 7 &&
    isCount(value[0]) &&
    isId(value[1]) &&
    isCount(value[2]) &&
    oneOf<LineKind>(LINE_KINDS)(value[3]) &&
    orNull(isText)(value[4]) &&
    isText(value[5]) &&
    isCount(value[6]);

const isLines = (value: unknown): value is ConsoleLine[] =>
    Array.isArray(value) && value.every(isConsoleLine);

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
        case 'server.status':
            return isServerStatus(data);
        case 'console':
            return (
                isLines(data.lines) &&
                typeof data.gap === 'boolean' &&
                typeof data.restarted === 'boolean' &&
                isCount(data.last)
            );
    }
};

const isEventData = (topic: Topic, data: unknown) => {
    switch (topic) {
        case 'status':
            return false;
        case 'audit':
            return isAuditEntry(data);
        case 'server.status':
            return isServerStatus(data);
        case 'console':
            return isRecord(data) && isLines(data.lines);
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
