import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import type { ServerState } from '../api/generated/ServerState';
import type { ServerStatus } from '../api/generated/ServerStatus';
import type { Messages } from '../i18n/en';

/** DESIGN.md §2.5: the LED and the colour of the word, never one without the other. */
export const STATE_LED = {
    online: 'led-live',
    degraded: 'led-warning',
    starting: 'led-pending',
    stopping: 'led-pending',
    stopped: '',
    hung: 'led-warning',
    crashed: 'led-down',
    crashLoop: 'led-down',
} satisfies Record<ServerState, string>;

export const STATE_INK = {
    online: 'text-ink',
    degraded: 'text-warning',
    starting: 'text-ink',
    stopping: 'text-ink',
    stopped: 'text-ink-2',
    hung: 'text-warning',
    crashed: 'text-danger-ink',
    crashLoop: 'text-danger-ink',
} satisfies Record<ServerState, string>;

/** A process runs: it takes commands, and its uptime counts. */
export const isRunning = (state: ServerState) =>
    state === 'starting' || state === 'online' || state === 'degraded' || state === 'hung';

/** Why the server is in its state, as a sentence; null when there is nothing to add to the word. */
export const causeText = (t: Messages, status: ServerStatus): string | null => {
    const words = t.server_state;
    const cause = status.cause;
    if (cause === null) {
        return null;
    }
    const by = (name: string) => (name === 'observer' ? words.observer : name);
    switch (cause.kind) {
        case 'requested':
            if (status.state === 'stopping') {
                return words.stopping(by(cause.by));
            }
            return status.state === 'stopped' ? words.stopped(by(cause.by)) : words.started(by(cause.by));
        case 'restart':
            return status.state === 'stopping'
                ? words.restarting(by(cause.by))
                : words.restarted(by(cause.by));
        case 'recovery':
            return words.recovery;
        case 'scheduled':
            if (cause.by === 'observer') {
                return status.state === 'stopping' ? words.restartingOnSchedule : words.restartedOnSchedule;
            }
            return status.state === 'stopping'
                ? words.restartingAsPlanned(cause.by)
                : words.restartedAsPlanned(cause.by);
        case 'ready':
            return words.ready;
        case 'recovered':
            return words.recovered;
        case 'unhealthy':
            return words.unhealthy(cause.problems.map((problem) => words.problems[problem]));
        case 'hang':
            return cause.cause === 'noHeartbeat' ? words.hangNoHeartbeat : words.hangNoHttp;
        case 'exited':
            return words.exited(cause.description);
        case 'fatalError':
            return words.fatalError(cause.message);
        case 'bindError':
            return words.bindError(cause.address);
        case 'licenceRejected':
            return words.licenceRejected;
        case 'consoleQuit':
            return words.consoleQuit(by(cause.by));
        case 'shutdown':
            return words.shutdown;
        case 'crashLoop':
            return words.crashLoop(cause.failures);
        case 'spawnFailed':
            return words.spawnFailed(cause.reason);
    }
};

/**
 * Whether the command line can send now, and what its field says otherwise. FXServer takes commands
 * until its process ends, so a stop leaves the line open; while the state is not known, Observer
 * answers for the server.
 */
export const commandLine = (
    t: Messages,
    status: ServerStatus | null,
): { open: boolean; note: string | null } => {
    const words = t.console;
    if (status === null) {
        return { open: true, note: words.stateUnknown };
    }
    if (isRunning(status.state)) {
        return { open: true, note: null };
    }
    switch (status.state) {
        case 'stopping':
            return { open: true, note: words.stopping };
        case 'stopped':
            return { open: false, note: words.stopped };
        default:
            return { open: false, note: words.notRunning };
    }
};

/**
 * What the Overview says of the build, first that applies: a probation, a return to the last
 * healthy build, the build the next start or the next scheduled restart runs, a probation passed
 * during this run. Null when there is nothing to add to the build's number.
 */
export const buildFact = (
    t: Messages,
    status: ServerStatus,
    builds: ArtifactsStatus | null,
    clock: (instant: string) => string,
): string | null => {
    const running = builds?.running ?? null;
    if (builds === null || running === null) {
        return null;
    }
    const build = String(running);
    const words = t.overview;
    if (builds.probation !== null && builds.probation.build === running) {
        return builds.probation.passesAt === null
            ? words.buildProbationWaiting(build)
            : words.buildProbation(build, clock(builds.probation.passesAt));
    }
    const notice = builds.notice;
    if (notice?.kind === 'rolledBack' && notice.other === running) {
        return words.buildBack(build, String(notice.build));
    }
    if (builds.selected !== null && builds.selected !== running) {
        return words.buildNextStart(build, String(builds.selected));
    }
    if (builds.pending !== null) {
        return words.buildPending(build, String(builds.pending.build));
    }
    const healthy = builds.lastHealthy;
    if (
        healthy !== null &&
        healthy.build === running &&
        status.startedAt !== null &&
        Date.parse(healthy.since) >= Date.parse(status.startedAt)
    ) {
        return words.buildPassed(build, clock(healthy.since));
    }
    return null;
};

export type Action = 'start' | 'stop' | 'restart';

/** Why `action` cannot be taken now, or null when it can. */
export const blocked = (
    t: Messages,
    action: Action,
    status: ServerStatus | null,
    permitted: boolean,
): string | null => {
    const words = t.controls;
    if (!permitted) {
        return words.notPermitted;
    }
    if (status === null) {
        return words.unknown;
    }
    if (status.state === 'stopping') {
        return words.stopping;
    }
    const running = isRunning(status.state);
    if (action === 'stop') {
        return status.state === 'stopped' ? words.notRunning : null;
    }
    if (action === 'start' && running) {
        return words.running;
    }
    if (action === 'restart' && !running) {
        return words.notRunning;
    }
    if (status.launch === 'notConfigured') {
        return words.notConfigured;
    }
    if (status.launch === 'noBuild') {
        return words.noBuild;
    }
    return status.launch === 'invalid' ? words.invalid : null;
};
