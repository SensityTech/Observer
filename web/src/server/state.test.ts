import { describe, expect, it } from 'vitest';

import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { DEFAULT_LAUNCH } from '../api/launch';
import { ARTIFACTS } from '../artifacts/fixtures';
import { en } from '../i18n/en';
import { blocked, buildFact, causeText, commandLine, hangAdvice } from './state';

const status = (overrides: Partial<ServerStatus>): ServerStatus => ({
    state: 'stopped',
    cause: null,
    since: '2026-10-28T14:00:00Z',
    startedAt: null,
    generation: 0,
    problems: [],
    build: null,
    players: null,
    restartAt: null,
    launch: 'ready',
    name: null,
    port: null,
    start: null,
    ...overrides,
});

describe('blocked', () => {
    it('should let each action through only in the states it fits', () => {
        const stopped = status({});
        const online = status({ state: 'online' });
        const crashed = status({ state: 'crashed' });
        const stopping = status({ state: 'stopping' });

        expect(blocked(en, 'start', stopped, true)).toBeNull();
        expect(blocked(en, 'start', crashed, true)).toBeNull();
        expect(blocked(en, 'start', online, true)).toBe(en.controls.running);
        expect(blocked(en, 'restart', online, true)).toBeNull();
        expect(blocked(en, 'restart', stopped, true)).toBe(en.controls.notRunning);
        expect(blocked(en, 'stop', online, true)).toBeNull();
        expect(blocked(en, 'stop', crashed, true)).toBeNull();
        expect(blocked(en, 'stop', stopped, true)).toBe(en.controls.notRunning);
        for (const action of ['start', 'stop', 'restart'] as const) {
            expect(blocked(en, action, stopping, true)).toBe(en.controls.stopping);
        }
    });

    it('should say what is missing before the state', () => {
        expect(blocked(en, 'stop', status({ state: 'online' }), false)).toBe(en.controls.notPermitted);
        expect(blocked(en, 'start', null, true)).toBe(en.controls.unknown);
        expect(blocked(en, 'start', status({ launch: 'notConfigured' }), true)).toBe(
            en.controls.notConfigured,
        );
        expect(blocked(en, 'restart', status({ state: 'online', launch: 'invalid' }), true)).toBe(
            en.controls.invalid,
        );
        expect(blocked(en, 'stop', status({ state: 'online', launch: 'invalid' }), true)).toBeNull();
    });
});

describe('commandLine', () => {
    it('should stay open while FXServer runs, stops, or its state is not known', () => {
        expect(commandLine(en, status({ state: 'online' }))).toEqual({ open: true, note: null });
        expect(commandLine(en, status({ state: 'hung' }))).toEqual({ open: true, note: null });
        expect(commandLine(en, status({ state: 'stopping' }))).toEqual({
            open: true,
            note: en.console.stopping,
        });
        expect(commandLine(en, null)).toEqual({ open: true, note: en.console.stateUnknown });
    });

    it('should close with the reason once no process is left', () => {
        expect(commandLine(en, status({ state: 'stopped' }))).toEqual({
            open: false,
            note: en.console.stopped,
        });
        for (const state of ['crashed', 'crashLoop'] as const) {
            expect(commandLine(en, status({ state }))).toEqual({ open: false, note: en.console.notRunning });
        }
    });
});

describe('causeText', () => {
    it('should word who asked by what the server does now', () => {
        const by = { kind: 'requested', by: 'ada' } as const;

        expect(causeText(en, status({ state: 'starting', cause: by }))).toBe('Started by ada.');
        expect(causeText(en, status({ state: 'stopped', cause: by }))).toBe('Stopped by ada.');
        expect(causeText(en, status({ state: 'stopping', cause: { kind: 'restart', by: 'ada' } }))).toBe(
            'Restarting, as ada asked: resources are told first, then FXServer quits and starts again.',
        );
        expect(causeText(en, status({ state: 'starting', cause: { kind: 'restart', by: 'ada' } }))).toBe(
            'Restarted by ada.',
        );
        expect(
            causeText(en, status({ state: 'starting', cause: { kind: 'requested', by: 'observer' } })),
        ).toBe('Started by Observer.');
        // `local` is someone at the host's own status window, who signs in to nothing.
        expect(causeText(en, status({ state: 'stopped', cause: { kind: 'requested', by: 'local' } }))).toBe(
            'Stopped by someone at the host.',
        );
        expect(causeText(en, status({ state: 'stopping', cause: { kind: 'restart', by: 'local' } }))).toBe(
            'Restarting, as someone at the host asked: resources are told first, then FXServer quits and starts again.',
        );
        expect(
            causeText(
                en,
                status({
                    state: 'degraded',
                    cause: { kind: 'unhealthy', problems: ['heartbeatLate', 'httpLate'] },
                }),
            ),
        ).toBe('Degraded: the bridge heartbeat is late and its HTTP endpoint answers late.');
        expect(causeText(en, status({ state: 'crashLoop', cause: { kind: 'crashLoop', failures: 5 } }))).toBe(
            '5 failures in a row: automatic restarts are off until someone starts FXServer.',
        );
        expect(causeText(en, status({ cause: { kind: 'spawnFailed', reason: 'missingData' } }))).toBe(
            'FXServer could not start: the server data folder does not exist.',
        );
        expect(causeText(en, status({}))).toBeNull();
    });

    it('should tell a restart of the schedule from a single one someone planned', () => {
        const schedule = { kind: 'scheduled', by: 'observer' } as const;
        const planned = { kind: 'scheduled', by: 'ada' } as const;

        expect(causeText(en, status({ state: 'stopping', cause: schedule }))).toBe(
            'Restarting on schedule: resources are told first, then FXServer quits and starts again.',
        );
        expect(causeText(en, status({ state: 'starting', cause: schedule }))).toBe('Restarted on schedule.');
        expect(causeText(en, status({ state: 'stopping', cause: planned }))).toBe(
            'Restarting, as ada planned: resources are told first, then FXServer quits and starts again.',
        );
        expect(causeText(en, status({ state: 'starting', cause: planned }))).toBe(
            'Restarted as ada planned.',
        );
    });
});

describe('hangAdvice', () => {
    const hung = status({ state: 'hung', cause: { kind: 'hang', cause: 'noHeartbeat' } });

    it('should name the delay that ran out, and where to raise it, for either silence', () => {
        const advice = hangAdvice(en, hung, { ...DEFAULT_LAUNCH, hangTimeoutSeconds: 900 });

        expect(advice).toBe(en.server_state.hangAdvice(15));
        expect(advice).toContain('FXServer stayed silent for 15 minutes');
        expect(advice).toContain('Server → Hang detection');
        expect(
            hangAdvice(en, { ...hung, cause: { kind: 'hang', cause: 'noHttp' } }, DEFAULT_LAUNCH),
        ).toContain('6 minutes');
        expect(hangAdvice(en, hung, { ...DEFAULT_LAUNCH, hangTimeoutSeconds: 60 })).toContain('1 minute,');
    });

    it('should say nothing for another cause, or to whoever cannot read the settings', () => {
        expect(hangAdvice(en, hung, null)).toBeNull();
        expect(
            hangAdvice(
                en,
                status({ state: 'crashed', cause: { kind: 'exited', description: 'exit code 1' } }),
                DEFAULT_LAUNCH,
            ),
        ).toBeNull();
        expect(hangAdvice(en, status({}), DEFAULT_LAUNCH)).toBeNull();
    });
});

/** The hour and minute of a UTC instant, as the schedule's zone would show it in UTC. */
const clock = (instant: string) => instant.slice(11, 16);

describe('buildFact', () => {
    const builds = structuredClone(ARTIFACTS) as unknown as ArtifactsStatus;
    const online = status({ state: 'online', startedAt: '2026-10-28T02:00:00Z', build: 37900 });
    const quiet: ArtifactsStatus = { ...builds, probation: null, pending: null, notice: null };

    it('should say first that the build runs on probation, until when', () => {
        expect(buildFact(en, online, builds, clock)).toBe(en.overview.buildProbation('37900', '02:38'));
        expect(
            buildFact(en, online, { ...builds, probation: { ...builds.probation!, passesAt: null } }, clock),
        ).toBe(en.overview.buildProbationWaiting('37900'));
    });

    it('should then say a return to the last healthy build, what starts next, or what waits', () => {
        const back: ArtifactsStatus = {
            ...quiet,
            running: 35245,
            selected: 35245,
            notice: {
                kind: 'rolledBack',
                build: 37900,
                other: 35245,
                reason: 'crashed',
                at: '2026-10-28T02:31:00Z',
            },
        };

        expect(buildFact(en, online, back, clock)).toBe(en.overview.buildBack('35245', '37900'));
        expect(buildFact(en, online, { ...quiet, selected: 37950 }, clock)).toBe(
            en.overview.buildNextStart('37900', '37950'),
        );
        expect(buildFact(en, online, { ...quiet, pending: builds.pending }, clock)).toBe(
            en.overview.buildPending('37900', '37950'),
        );
    });

    it('should say a probation passed only during the run that passed it', () => {
        const passed: ArtifactsStatus = {
            ...quiet,
            lastHealthy: { build: 37900, since: '2026-10-28T02:28:00Z' },
        };

        expect(buildFact(en, online, passed, clock)).toBe(en.overview.buildPassed('37900', '02:28'));
        expect(buildFact(en, { ...online, startedAt: '2026-10-28T03:00:00Z' }, passed, clock)).toBeNull();
        expect(buildFact(en, online, null, clock)).toBeNull();
    });
});
