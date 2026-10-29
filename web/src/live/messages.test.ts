import { describe, expect, it } from 'vitest';

import { ARTIFACTS } from '../artifacts/fixtures';
import { parseServerMessage } from './messages';

const parse = (message: unknown) => parseServerMessage(JSON.stringify(message));

const STATUS = {
    state: 'crashed',
    cause: { kind: 'exited', description: '0xC0000005 access violation' },
    since: '2026-10-28T14:02:07Z',
    startedAt: null,
    generation: 3,
    problems: [],
    build: null,
    players: null,
    restartAt: '2026-10-28T14:02:17Z',
    launch: 'ready',
};

const ONLINE = {
    ...STATUS,
    state: 'online',
    cause: { kind: 'ready' },
    startedAt: '2026-10-28T14:02:18Z',
    build: 35245,
    players: { online: 187, max: 256 },
    restartAt: null,
};

const LINE = [1_761_660_127_000_001, 1_761_660_127_123, 3, 'command', 'ada', 'ensure chat', 0];

const SAMPLE = {
    ts: 1_761_660_125,
    state: 'online',
    values: { players: 187, slots: 256, fxsCpu: 182.5, mainTick: 6.8 },
    children: [{ pid: 4120, name: 'node.exe', cpu: null, workingSet: 52_428_800, private: 41_943_040 }],
    cores: 16,
};

const SCHEDULE = {
    settings: {
        enabled: true,
        mode: 'fixedTimes',
        times: ['04:00', '16:00'],
        intervalMinutes: 360,
        timeZone: null,
        warnings: [30, 15, 10, 5, 4, 3, 2, 1],
        skipAfterStart: 120,
    },
    version: 3,
    zone: 'Europe/Paris',
    next: { at: '2026-10-29T03:30:00Z', source: 'schedule', by: 'ada', delayedFrom: '2026-10-29T03:00:00Z' },
    upcoming: [
        {
            at: '2026-10-29T03:00:00Z',
            time: '04:00',
            skipped: null,
            delayed: { to: '2026-10-29T03:30:00Z', by: 'ada' },
        },
        {
            at: '2026-10-29T15:00:00Z',
            time: '16:00',
            skipped: { kind: 'recentStart', startedAt: '2026-10-29T14:10:00Z' },
            delayed: null,
        },
    ],
    oneOff: null,
};

describe('parseServerMessage', () => {
    it('should accept every message of the contract', () => {
        for (const message of [
            { type: 'hello', connectionId: 3, contract: 1 },
            {
                type: 'snapshot',
                topic: 'status',
                data: { version: '0.1.0', startedAt: '2026-10-28T09:00:00Z' },
            },
            { type: 'snapshot', topic: 'audit', data: { latestId: null } },
            { type: 'snapshot', topic: 'server.status', data: STATUS },
            { type: 'event', topic: 'server.status', data: ONLINE },
            {
                type: 'event',
                topic: 'server.status',
                data: { ...STATUS, state: 'stopped', cause: { kind: 'spawnFailed', reason: 'notABuild' } },
            },
            {
                type: 'snapshot',
                topic: 'console',
                data: { lines: [LINE], gap: true, restarted: false, last: LINE[0] },
            },
            {
                type: 'event',
                topic: 'console',
                data: { lines: [LINE, [...LINE.slice(0, 3), 'stdout', null, 'x', 4]] },
            },
            { type: 'snapshot', topic: 'server.schedule', data: SCHEDULE },
            {
                type: 'event',
                topic: 'server.schedule',
                data: { ...SCHEDULE, oneOff: { at: '2026-10-28T15:07:00Z', by: 'grace' }, version: null },
            },
            { type: 'snapshot', topic: 'server.artifacts', data: ARTIFACTS },
            {
                type: 'event',
                topic: 'server.artifacts',
                data: {
                    ...ARTIFACTS,
                    job: { ...ARTIFACTS.job, state: 'failed', failure: { code: 'httpStatus', status: 503 } },
                    notice: {
                        kind: 'rolledBack',
                        build: 37900,
                        other: 35245,
                        reason: 'crashed',
                        at: '2026-10-28T02:31:00Z',
                    },
                },
            },
            { type: 'resync', topic: 'audit' },
            { type: 'snapshot', topic: 'metrics.live', data: { latest: null } },
            { type: 'snapshot', topic: 'metrics.live', data: { latest: SAMPLE } },
            { type: 'event', topic: 'metrics.live', data: { ...SAMPLE, values: {}, children: [] } },
            { type: 'event', topic: 'metrics.live', data: { ...SAMPLE, cores: null } },
            { type: 'resync', topic: 'metrics.live' },
            { type: 'unsubscribed', topic: 'status', reason: 'forbidden' },
            { type: 'pong' },
            { type: 'error', code: 'invalid_request' },
        ]) {
            expect(parse(message)).toEqual(message);
        }
    });

    it('should refuse what only looks like it', () => {
        for (const message of [
            null,
            [],
            { type: 'hello', connectionId: '3', contract: 1 },
            { type: 'snapshot', topic: 'status', data: { version: '0.1.0', startedAt: 'yesterday' } },
            { type: 'snapshot', topic: 'audit' },
            { type: 'unsubscribed', topic: 'audit', reason: 'bored' },
            { type: 'resync', topic: 'metrics.history' },
            { type: 'snapshot', topic: 'metrics.live', data: {} },
            { type: 'event', topic: 'metrics.live', data: { ...SAMPLE, values: { players: '187' } } },
            { type: 'event', topic: 'metrics.live', data: { ...SAMPLE, values: { fxs_cpu: 3 } } },
            { type: 'event', topic: 'metrics.live', data: { ...SAMPLE, values: { players: null } } },
            { type: 'event', topic: 'metrics.live', data: { ...SAMPLE, cores: 0 } },
            {
                type: 'event',
                topic: 'metrics.live',
                data: { ...SAMPLE, children: [{ ...SAMPLE.children[0], workingSet: -1 }] },
            },
            { type: 'snapshot', topic: 'server.status', data: { ...STATUS, state: 'exploded' } },
            { type: 'snapshot', topic: 'server.status', data: { ...STATUS, cause: { kind: 'exited' } } },
            {
                type: 'snapshot',
                topic: 'server.status',
                data: { ...STATUS, cause: { kind: 'spawnFailed', reason: 'the folder D:/prod is missing' } },
            },
            {
                type: 'event',
                topic: 'server.status',
                data: { ...ONLINE, players: { online: -1, max: null } },
            },
            { type: 'snapshot', topic: 'console', data: { lines: [LINE], gap: false } },
            {
                type: 'snapshot',
                topic: 'console',
                data: { lines: [LINE], gap: false, restarted: false, last: -1 },
            },
            { type: 'event', topic: 'console', data: { lines: [LINE.slice(0, 6)] } },
            {
                type: 'event',
                topic: 'console',
                data: { lines: [[...LINE.slice(0, 3), 'stdin', null, 'x', 0]] },
            },
            {
                type: 'snapshot',
                topic: 'server.schedule',
                data: { ...SCHEDULE, settings: { ...SCHEDULE.settings, mode: 'weekly' } },
            },
            {
                type: 'event',
                topic: 'server.schedule',
                data: { ...SCHEDULE, upcoming: [{ ...SCHEDULE.upcoming[0], skipped: { kind: 'manual' } }] },
            },
            {
                type: 'snapshot',
                topic: 'server.artifacts',
                data: { ...ARTIFACTS, job: { ...ARTIFACTS.job, step: 'unpacking' } },
            },
            {
                type: 'event',
                topic: 'server.artifacts',
                data: { ...ARTIFACTS, installed: [{ ...ARTIFACTS.installed[0], build: 12 }] },
            },
            { type: 'snapshot', topic: 'server.artifacts', data: { ...ARTIFACTS, disk: { free: '1 GB' } } },
            { type: 'shutdown' },
        ]) {
            expect(parse(message)).toBeNull();
        }
        expect(parseServerMessage('{"type":')).toBeNull();
    });
});
