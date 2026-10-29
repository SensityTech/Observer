import type { ActorKind } from '../api/generated/ActorKind';
import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import type { AuditAction } from '../api/generated/AuditAction';
import type { AuditEntry } from '../api/generated/AuditEntry';
import type { BrokenReason } from '../api/generated/BrokenReason';
import type { ConsoleLine } from '../api/generated/ConsoleLine';
import type { HangCause } from '../api/generated/HangCause';
import type { HealthProblem } from '../api/generated/HealthProblem';
import type { JobKind } from '../api/generated/JobKind';
import type { JobState } from '../api/generated/JobState';
import type { JobStep } from '../api/generated/JobStep';
import type { JobWarning } from '../api/generated/JobWarning';
import type { LaunchReadiness } from '../api/generated/LaunchReadiness';
import type { LineKind } from '../api/generated/LineKind';
import type { LiveSample } from '../api/generated/LiveSample';
import type { NoticeKind } from '../api/generated/NoticeKind';
import type { NoticeReason } from '../api/generated/NoticeReason';
import type { Outcome } from '../api/generated/Outcome';
import type { RestartSource } from '../api/generated/RestartSource';
import type { ScheduleMode } from '../api/generated/ScheduleMode';
import type { ScheduleStatus } from '../api/generated/ScheduleStatus';
import type { Series } from '../api/generated/Series';
import type { ServerMessage } from '../api/generated/ServerMessage';
import type { ServerState } from '../api/generated/ServerState';
import type { ServerStatus } from '../api/generated/ServerStatus';
import type { SpawnFailure } from '../api/generated/SpawnFailure';
import type { StateCause } from '../api/generated/StateCause';
import type { Topic } from '../api/generated/Topic';

// What arrives on the socket is untrusted JSON until it matches the contract field by field. The
// records below list every value of the generated unions: one the Rust side adds fails the type check
// here until it is listed.

const TOPICS = {
    status: true,
    audit: true,
    'server.status': true,
    console: true,
    'metrics.live': true,
    'server.schedule': true,
    'server.artifacts': true,
} satisfies Record<Topic, true>;

export const SERIES = {
    players: true,
    slots: true,
    pingAvg: true,
    pingP95: true,
    pingMax: true,
    joins: true,
    leaves: true,
    mainTick: true,
    networkTick: true,
    syncTick: true,
    mainOverBudget: true,
    networkOverBudget: true,
    syncOverBudget: true,
    hitchesMain: true,
    hitchesNetwork: true,
    hitchesSync: true,
    barks: true,
    hitchMax: true,
    resources: true,
    jsHeapUsed: true,
    jsHeapTotal: true,
    fxsCpu: true,
    fxsWorkingSet: true,
    fxsPrivate: true,
    fxsThreads: true,
    fxsHandles: true,
    fxsRead: true,
    fxsWrite: true,
    children: true,
    childrenCpu: true,
    childrenWorkingSet: true,
    childrenPrivate: true,
    hostCpu: true,
    hostMemoryUsed: true,
    hostMemoryTotal: true,
    hostDiskFree: true,
    hostDiskTotal: true,
    hostReceive: true,
    hostSend: true,
    observerCpu: true,
    observerWorkingSet: true,
    observerPrivate: true,
    stateDatabase: true,
    metricsDatabase: true,
    dashboardClients: true,
    sampleTime: true,
} satisfies Record<Series, true>;

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
    build_install_started: true,
    build_install_cancelled: true,
    build_install_finished: true,
    build_selected: true,
    pending_build_set: true,
    pending_build_cancelled: true,
    pending_build_applied: true,
    pending_build_dropped: true,
    probation_passed: true,
    probation_failed: true,
    build_rolled_back: true,
    build_deleted: true,
    artifact_settings_changed: true,
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

const READINESS = {
    ready: true,
    notConfigured: true,
    invalid: true,
    noBuild: true,
} satisfies Record<LaunchReadiness, true>;

const LINE_KINDS = { stdout: true, stderr: true, command: true, observer: true } satisfies Record<
    LineKind,
    true
>;

const SCHEDULE_MODES = { fixedTimes: true, interval: true } satisfies Record<ScheduleMode, true>;

const RESTART_SOURCES = { schedule: true, oneOff: true } satisfies Record<RestartSource, true>;

const BROKEN_REASONS = {
    noManifest: true,
    badManifest: true,
    link: true,
    layout: true,
    archiveChanged: true,
} satisfies Record<BrokenReason, true>;

const NOTICE_KINDS = { rolledBack: true, probationFailed: true, pendingDropped: true } satisfies Record<
    NoticeKind,
    true
>;

const NOTICE_REASONS = {
    crashed: true,
    hung: true,
    disabled: true,
    noHealthyBuild: true,
    broken: true,
    kvp: true,
    notInstalled: true,
    replaced: true,
} satisfies Record<NoticeReason, true>;

const JOB_KINDS = { install: true, import: true } satisfies Record<JobKind, true>;

const JOB_STEPS = {
    preCheck: true,
    download: true,
    verify: true,
    extract: true,
    copy: true,
    validate: true,
    commit: true,
} satisfies Record<JobStep, true>;

const JOB_STATES = { running: true, installed: true, failed: true, cancelled: true } satisfies Record<
    JobState,
    true
>;

const JOB_WARNINGS = { longPath: true, freeSpaceUnknown: true } satisfies Record<JobWarning, true>;

type Shape = 'text' | 'count' | 'problems' | 'hang' | 'spawnFailure';

/** What each kind of cause carries besides its kind. */
const CAUSE_FIELDS = {
    requested: { by: 'text' },
    restart: { by: 'text' },
    recovery: {},
    scheduled: { by: 'text' },
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

const isFiniteNumber = (value: unknown): value is number =>
    typeof value === 'number' && Number.isFinite(value);

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

const isCounts = (value: unknown): value is number[] => Array.isArray(value) && value.every(isCount);

const isScheduleSettings = (value: unknown) =>
    isRecord(value) &&
    typeof value.enabled === 'boolean' &&
    oneOf<ScheduleMode>(SCHEDULE_MODES)(value.mode) &&
    Array.isArray(value.times) &&
    value.times.every(isText) &&
    isCount(value.intervalMinutes) &&
    orNull(isText)(value.timeZone) &&
    isCounts(value.warnings) &&
    isCount(value.skipAfterStart);

const isSkipped = (value: unknown) =>
    value === null ||
    (isRecord(value) &&
        ((value.kind === 'manual' && isText(value.by)) ||
            (value.kind === 'recentStart' && isTimestamp(value.startedAt))));

const isOccurrence = (value: unknown) =>
    isRecord(value) &&
    isTimestamp(value.at) &&
    orNull(isText)(value.time) &&
    isSkipped(value.skipped) &&
    (value.delayed === null ||
        (isRecord(value.delayed) && isTimestamp(value.delayed.to) && isText(value.delayed.by)));

const isNextRestart = (value: unknown) =>
    isRecord(value) &&
    isTimestamp(value.at) &&
    oneOf<RestartSource>(RESTART_SOURCES)(value.source) &&
    orNull(isText)(value.by) &&
    orNull(isTimestamp)(value.delayedFrom);

export const isScheduleStatus = (value: unknown): value is ScheduleStatus =>
    isRecord(value) &&
    isScheduleSettings(value.settings) &&
    orNull(isId)(value.version) &&
    isText(value.zone) &&
    (value.next === null || isNextRestart(value.next)) &&
    Array.isArray(value.upcoming) &&
    value.upcoming.every(isOccurrence) &&
    (value.oneOff === null ||
        (isRecord(value.oneOff) && isTimestamp(value.oneOff.at) && isText(value.oneOff.by)));

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

/** Every key a known series, every value a finite number. */
const isSeriesValues = (value: unknown) =>
    isRecord(value) &&
    Object.entries(value).every(
        ([series, measured]) => Object.hasOwn(SERIES, series) && isFiniteNumber(measured),
    );

const isChildProcess = (value: unknown) =>
    isRecord(value) &&
    isCount(value.pid) &&
    isText(value.name) &&
    orNull(isFiniteNumber)(value.cpu) &&
    isCount(value.workingSet) &&
    isCount(value.private);

export const isLiveSample = (value: unknown): value is LiveSample =>
    isRecord(value) &&
    isId(value.ts) &&
    oneOf<ServerState>(SERVER_STATES)(value.state) &&
    isSeriesValues(value.values) &&
    Array.isArray(value.children) &&
    value.children.every(isChildProcess) &&
    (value.cores === null || (isCount(value.cores) && value.cores > 0));

const isBuild = (value: unknown): value is number => isCount(value) && value >= 1000;

const isInstalledBuild = (value: unknown) =>
    isRecord(value) &&
    isBuild(value.build) &&
    (value.origin === null || value.origin === 'download' || value.origin === 'import') &&
    orNull(isText)(value.commit) &&
    orNull(isText)(value.branch) &&
    orNull(isTimestamp)(value.installedAt) &&
    orNull(isText)(value.installedBy) &&
    isCount(value.bytes) &&
    orNull(isCount)(value.files) &&
    isRecord(value.health) &&
    (value.health.kind === 'valid' ||
        (value.health.kind === 'broken' && oneOf<BrokenReason>(BROKEN_REASONS)(value.health.reason)));

const isJob = (value: unknown): value is Record<string, unknown> =>
    isRecord(value) &&
    isCount(value.id) &&
    oneOf<JobKind>(JOB_KINDS)(value.kind) &&
    isBuild(value.build) &&
    oneOf<JobStep>(JOB_STEPS)(value.step) &&
    oneOf<JobState>(JOB_STATES)(value.state) &&
    isText(value.startedBy) &&
    isTimestamp(value.startedAt) &&
    orNull(isTimestamp)(value.finishedAt) &&
    (value.download === null ||
        (isRecord(value.download) &&
            isCount(value.download.received) &&
            isCount(value.download.total) &&
            isCount(value.download.rate))) &&
    (value.entries === null ||
        (isRecord(value.entries) && isCount(value.entries.done) && orNull(isCount)(value.entries.total))) &&
    typeof value.cancelRequested === 'boolean' &&
    (value.failure === null || (isRecord(value.failure) && isText(value.failure.code))) &&
    Array.isArray(value.warnings) &&
    value.warnings.every(oneOf<JobWarning>(JOB_WARNINGS));

export const isArtifactsStatus = (value: unknown): value is ArtifactsStatus =>
    isRecord(value) &&
    (value.platform === 'windows' || value.platform === 'linux') &&
    Array.isArray(value.installed) &&
    value.installed.every(isInstalledBuild) &&
    orNull(isBuild)(value.selected) &&
    (value.pending === null ||
        (isRecord(value.pending) &&
            isBuild(value.pending.build) &&
            isText(value.pending.by) &&
            isTimestamp(value.pending.at))) &&
    orNull(isBuild)(value.running) &&
    (value.lastHealthy === null ||
        (isRecord(value.lastHealthy) &&
            isBuild(value.lastHealthy.build) &&
            isTimestamp(value.lastHealthy.since))) &&
    (value.probation === null ||
        (isRecord(value.probation) &&
            isBuild(value.probation.build) &&
            orNull(isBuild)(value.probation.from) &&
            orNull(isTimestamp)(value.probation.readyAt) &&
            orNull(isTimestamp)(value.probation.passesAt))) &&
    orNull(isBuild)(value.highestRun) &&
    (value.notice === null ||
        (isRecord(value.notice) &&
            oneOf<NoticeKind>(NOTICE_KINDS)(value.notice.kind) &&
            isBuild(value.notice.build) &&
            orNull(isBuild)(value.notice.other) &&
            oneOf<NoticeReason>(NOTICE_REASONS)(value.notice.reason) &&
            isTimestamp(value.notice.at))) &&
    orNull(isJob)(value.job) &&
    isRecord(value.settings) &&
    isCount(value.settings.keepBuilds) &&
    typeof value.settings.autoRollback === 'boolean' &&
    Array.isArray(value.settings.branches) &&
    value.settings.branches.every(isText) &&
    orNull(isCount)(value.version) &&
    typeof value.legacyFolder === 'boolean' &&
    orNull(isCount)(value.longPath) &&
    (value.disk === null || (isRecord(value.disk) && isCount(value.disk.free) && isCount(value.disk.total)));

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
        case 'metrics.live':
            return orNull(isLiveSample)(data.latest);
        case 'server.schedule':
            return isScheduleStatus(data);
        case 'server.artifacts':
            return isArtifactsStatus(data);
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
        case 'metrics.live':
            return isLiveSample(data);
        case 'server.schedule':
            return isScheduleStatus(data);
        case 'server.artifacts':
            return isArtifactsStatus(data);
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
