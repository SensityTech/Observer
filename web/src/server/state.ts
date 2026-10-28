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
        case 'recovery':
            return words.recovery;
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
            return words.spawnFailed(cause.error);
    }
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
    return status.launch === 'invalid' ? words.invalid : null;
};
