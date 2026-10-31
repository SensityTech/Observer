import type { ActorKind } from '../api/generated/ActorKind';
import type { AlertKind } from '../api/generated/AlertKind';
import type { AlertList } from '../api/generated/AlertList';
import type { AnalysisState } from '../api/generated/AnalysisState';
import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import type { AuditAction } from '../api/generated/AuditAction';
import type { AuditEntry } from '../api/generated/AuditEntry';
import type { BrokenReason } from '../api/generated/BrokenReason';
import type { ConsoleLine } from '../api/generated/ConsoleLine';
import type { Crash } from '../api/generated/Crash';
import type { CrashFileKind } from '../api/generated/CrashFileKind';
import type { CrashFolder } from '../api/generated/CrashFolder';
import type { CrashKind } from '../api/generated/CrashKind';
import type { Destination } from '../api/generated/Destination';
import type { DiagnosticsEvent } from '../api/generated/DiagnosticsEvent';
import type { DiagnosticsSnapshot } from '../api/generated/DiagnosticsSnapshot';
import type { HangCause } from '../api/generated/HangCause';
import type { HealthProblem } from '../api/generated/HealthProblem';
import type { JobKind } from '../api/generated/JobKind';
import type { JobState } from '../api/generated/JobState';
import type { JobStep } from '../api/generated/JobStep';
import type { JobWarning } from '../api/generated/JobWarning';
import type { LaunchReadiness } from '../api/generated/LaunchReadiness';
import type { LineKind } from '../api/generated/LineKind';
import type { Listener } from '../api/generated/Listener';
import type { ListenerRole } from '../api/generated/ListenerRole';
import type { LiveSample } from '../api/generated/LiveSample';
import type { NoticeKind } from '../api/generated/NoticeKind';
import type { NoticeReason } from '../api/generated/NoticeReason';
import type { Outcome } from '../api/generated/Outcome';
import type { ProcessKind } from '../api/generated/ProcessKind';
import type { Protocol } from '../api/generated/Protocol';
import type { RestartSource } from '../api/generated/RestartSource';
import type { ScheduleMode } from '../api/generated/ScheduleMode';
import type { ScheduleStatus } from '../api/generated/ScheduleStatus';
import type { Scope } from '../api/generated/Scope';
import type { Series } from '../api/generated/Series';
import type { ServerMessage } from '../api/generated/ServerMessage';
import type { ServerState } from '../api/generated/ServerState';
import type { ServerStatus } from '../api/generated/ServerStatus';
import type { Service } from '../api/generated/Service';
import type { SpawnFailure } from '../api/generated/SpawnFailure';
import type { StateCause } from '../api/generated/StateCause';
import type { SyscallGroup } from '../api/generated/SyscallGroup';
import type { ThreadRun } from '../api/generated/ThreadRun';
import type { Topic } from '../api/generated/Topic';
import type { TreeProcess } from '../api/generated/TreeProcess';

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
    'server.alerts': true,
    'server.diagnostics': true,
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
    fxsKernel: true,
    fxsFaults: true,
    fxsHardFaults: true,
    fxsWorkingSetPeak: true,
    mainThreadCpu: true,
    networkThreadCpu: true,
    syncThreadCpu: true,
    jsThreadsCpu: true,
    monoThreadsCpu: true,
    otherThreadsCpu: true,
    treeCpu: true,
    treeStarted: true,
    treePrivatePeak: true,
    hostClock: true,
    hostClockMax: true,
    hostClockLimit: true,
    hostDiskLatency: true,
    hostDiskTransfers: true,
    hostSteal: true,
    hostPressureCpu: true,
    hostPressureMemory: true,
    hostPressureIo: true,
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
    observer_stopped: true,
    tree_process_started: true,
    hang_snapshot_taken: true,
    crash_file_downloaded: true,
    crash_files_deleted: true,
    tree_records_full: true,
    player_forgotten: true,
    players_imported: true,
    players_retention_changed: true,
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

const PROCESS_KINDS = {
    dumpServer: true,
    consoleHost: true,
    node: true,
    shell: true,
    other: true,
} satisfies Record<ProcessKind, true>;

const PROTOCOLS = { tcp: true, udp: true } satisfies Record<Protocol, true>;

const SCOPES = { loopback: true, private: true, public: true } satisfies Record<Scope, true>;

const SERVICES = {
    mysql: true,
    postgresql: true,
    redis: true,
    mongodb: true,
    https: true,
    http: true,
    dns: true,
} satisfies Record<Service, true>;

const LISTENER_ROLES = { game: true } satisfies Record<ListenerRole, true>;

const CRASH_KINDS = { crash: true, hang: true, found: true } satisfies Record<CrashKind, true>;

const ANALYSIS_STATES = {
    pending: true,
    done: true,
    noDump: true,
    unreadable: true,
} satisfies Record<AnalysisState, true>;

const CRASH_FILE_KINDS = {
    minidump: true,
    fullDump: true,
    liveDump: true,
    hangDump: true,
    hangReport: true,
    other: true,
} satisfies Record<CrashFileKind, true>;

const THREAD_RUNS = {
    running: true,
    sleeping: true,
    diskWait: true,
    stopped: true,
    zombie: true,
    other: true,
} satisfies Record<ThreadRun, true>;

const SYSCALL_GROUPS = { lock: true, io: true, wait: true, sleep: true } satisfies Record<SyscallGroup, true>;

const ALERT_KINDS = { shell: true, crash: true, hang: true } satisfies Record<AlertKind, true>;

const isFlag = (value: unknown): value is boolean => typeof value === 'boolean';

const isListOf =
    <T>(check: (value: unknown) => value is T) =>
    (value: unknown): value is T[] =>
        Array.isArray(value) && value.every(check);

const isTreeProcess = (value: unknown): value is TreeProcess =>
    isRecord(value) &&
    isId(value.id) &&
    isTimestamp(value.runStarted) &&
    isCount(value.generation) &&
    isCount(value.pid) &&
    (value.parent === null ||
        (isRecord(value.parent) && isCount(value.parent.pid) && orNull(isText)(value.parent.name))) &&
    orNull(isText)(value.image) &&
    orNull(isText)(value.name) &&
    orNull(isText)(value.commandLine) &&
    isFlag(value.truncated) &&
    oneOf<ProcessKind>(PROCESS_KINDS)(value.kind) &&
    isTimestamp(value.startedAt) &&
    isTimestamp(value.lastStartedAt) &&
    orNull(isTimestamp)(value.endedAt) &&
    isCount(value.count);

const isDestination = (value: unknown): value is Destination =>
    isRecord(value) &&
    oneOf<Protocol>(PROTOCOLS)(value.protocol) &&
    isText(value.address) &&
    isCount(value.port) &&
    isText(value.process) &&
    oneOf<Scope>(SCOPES)(value.scope) &&
    orNull(oneOf<Service>(SERVICES))(value.service) &&
    isCount(value.open) &&
    isCount(value.peak) &&
    isTimestamp(value.firstSeen) &&
    isTimestamp(value.lastSeen);

const isListener = (value: unknown): value is Listener =>
    isRecord(value) &&
    oneOf<Protocol>(PROTOCOLS)(value.protocol) &&
    isText(value.address) &&
    isCount(value.port) &&
    isText(value.process) &&
    orNull(oneOf<ListenerRole>(LISTENER_ROLES))(value.role) &&
    isCount(value.inbound);

const isThreadState = (value: unknown) =>
    isRecord(value) &&
    isCount(value.tid) &&
    isText(value.name) &&
    oneOf<ThreadRun>(THREAD_RUNS)(value.state) &&
    orNull(isFiniteNumber)(value.cpu) &&
    orNull(isText)(value.syscall) &&
    orNull(oneOf<SyscallGroup>(SYSCALL_GROUPS))(value.syscallGroup);

const isFrame = (value: unknown) =>
    isRecord(value) && orNull(isText)(value.module) && isText(value.offset) && isFlag(value.scanned);

const isAnalysis = (value: unknown) =>
    isRecord(value) &&
    (value.exception === null ||
        (isRecord(value.exception) &&
            isText(value.exception.code) &&
            isText(value.exception.name) &&
            orNull(isText)(value.exception.address))) &&
    (value.thread === null ||
        (isRecord(value.thread) && isCount(value.thread.id) && orNull(isText)(value.thread.name))) &&
    Array.isArray(value.frames) &&
    value.frames.every(isFrame) &&
    orNull(isText)(value.module) &&
    Array.isArray(value.threads) &&
    value.threads.every(isThreadState) &&
    orNull(isFlag)(value.syscallsReadable);

const isCrashFile = (value: unknown) =>
    isRecord(value) &&
    isText(value.name) &&
    isCount(value.bytes) &&
    oneOf<CrashFileKind>(CRASH_FILE_KINDS)(value.kind) &&
    isFlag(value.kept);

const isCrash = (value: unknown): value is Crash =>
    isRecord(value) &&
    isId(value.id) &&
    isTimestamp(value.at) &&
    oneOf<CrashKind>(CRASH_KINDS)(value.kind) &&
    orNull(isTimestamp)(value.runStarted) &&
    orNull(isCount)(value.generation) &&
    orNull(isCount)(value.build) &&
    orNull(isText)(value.exit) &&
    orNull(isText)(value.exitCode) &&
    orNull(isId)(value.signal) &&
    orNull(oneOf<HangCause>(HANG_CAUSES))(value.hangCause) &&
    orNull(isText)(value.signature) &&
    oneOf<AnalysisState>(ANALYSIS_STATES)(value.analysisState) &&
    (value.analysis === null || isAnalysis(value.analysis)) &&
    orNull(isText)(value.watchdogStack) &&
    Array.isArray(value.files) &&
    value.files.every(isCrashFile) &&
    (value.group === null ||
        (isRecord(value.group) && isCount(value.group.count) && isTimestamp(value.group.firstAt))) &&
    isTimestamp(value.updatedAt);

const isCrashFolder = (value: unknown): value is CrashFolder =>
    isRecord(value) &&
    isCount(value.files) &&
    isCount(value.bytes) &&
    isCount(value.maxFiles) &&
    isCount(value.maxBytes) &&
    orNull(isCount)(value.free);

export const isDiagnosticsSnapshot = (value: unknown): value is DiagnosticsSnapshot =>
    isRecord(value) &&
    isListOf(isTreeProcess)(value.processes) &&
    isListOf(isDestination)(value.destinations) &&
    isListOf(isListener)(value.listeners) &&
    isListOf(isCrash)(value.crashes) &&
    isCrashFolder(value.crashFolder);

export const isDiagnosticsEvent = (value: unknown): value is DiagnosticsEvent => {
    if (!isRecord(value)) {
        return false;
    }
    switch (value.kind) {
        case 'process':
            return isTreeProcess(value.process);
        case 'connections':
            return isListOf(isDestination)(value.destinations) && isListOf(isListener)(value.listeners);
        case 'crash':
            return isCrash(value.crash);
        case 'crashFolder':
            return isCrashFolder(value.folder);
        default:
            return false;
    }
};

const isAlertCrash = (value: unknown) =>
    isRecord(value) &&
    (value.exception === null ||
        (isRecord(value.exception) && isText(value.exception.code) && isText(value.exception.name))) &&
    orNull(isText)(value.module) &&
    orNull(isText)(value.thread) &&
    orNull(isText)(value.exitCode) &&
    orNull(isId)(value.signal) &&
    isFlag(value.busy) &&
    orNull(isText)(value.waitingIn) &&
    isFlag(value.snapshot) &&
    isCount(value.groupCount);

const isAlert = (value: unknown) =>
    isRecord(value) &&
    isText(value.key) &&
    oneOf<AlertKind>(ALERT_KINDS)(value.kind) &&
    isTimestamp(value.at) &&
    isTimestamp(value.lastAt) &&
    isCount(value.count) &&
    orNull(isText)(value.name) &&
    orNull(isText)(value.parent) &&
    (value.crash === null || isAlertCrash(value.crash));

export const isAlertList = (value: unknown): value is AlertList =>
    isRecord(value) && Array.isArray(value.alerts) && value.alerts.every(isAlert) && isTimestamp(value.since);
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
        case 'server.alerts':
            return isAlertList(data);
        case 'server.diagnostics':
            return isDiagnosticsSnapshot(data);
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
        case 'server.alerts':
            return isAlertList(data);
        case 'server.diagnostics':
            return isDiagnosticsEvent(data);
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
