import { describe, expect, it } from 'vitest';

import type { ServerStatus } from '../api/generated/ServerStatus';
import { en } from '../i18n/en';
import { blocked, causeText, commandLine } from './state';

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
});
