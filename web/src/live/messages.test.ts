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
    start: null,
    stopHolders: [],
    stop: null,
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

const STARTING = {
    ...ONLINE,
    state: 'starting',
    cause: { kind: 'requested', by: 'ada' },
    build: null,
    players: null,
    start: {
        started: 152,
        failed: 1,
        found: 391,
        planned: 380,
        last: { name: 'core', at: '2026-10-28T14:04:10Z', warnings: 1 },
        at: {
            file: 'server.cfg',
            line: 226,
            target: '[clothes]',
            since: '2026-10-28T14:04:10Z',
            done: 12,
            of: 150,
            resource: null,
        },
        slowest: [{ name: 'maps', ms: 5700 }],
        stuckAt: '2026-10-28T14:10:10Z',
        extensionMinutes: 15,
    },
};

const STOPPING = {
    ...ONLINE,
    state: 'stopping',
    cause: { kind: 'restart', by: 'ada' },
    stopHolders: ['esx_vehicleshop', 'qb-inventory'],
    stop: {
        since: '2026-10-28T16:00:00Z',
        limitAt: '2026-10-28T16:01:00Z',
        holding: ['qb-inventory'],
        released: [{ resource: 'esx_vehicleshop', afterMs: 2400 }],
    },
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
            { type: 'event', topic: 'server.status', data: { ...ONLINE, stopHolders: ['qb-inventory'] } },
            { type: 'event', topic: 'server.status', data: STOPPING },
            { type: 'event', topic: 'server.status', data: STARTING },
            {
                type: 'event',
                topic: 'server.status',
                data: {
                    ...STARTING,
                    start: {
                        ...STARTING.start,
                        found: null,
                        planned: null,
                        last: null,
                        at: null,
                        stuckAt: null,
                    },
                },
            },
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
            { type: 'unsubscribed', topic: 'metrics.live', reason: 'disabled' },
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
            { type: 'event', topic: 'server.status', data: { ...ONLINE, stopHolders: [7] } },
            {
                type: 'event',
                topic: 'server.status',
                data: { ...STOPPING, stop: { ...STOPPING.stop, limitAt: 'soon' } },
            },
            {
                type: 'event',
                topic: 'server.status',
                data: { ...STOPPING, stop: { ...STOPPING.stop, released: [{ resource: 'chat' }] } },
            },
            {
                type: 'event',
                topic: 'server.status',
                data: { ...STARTING, start: { ...STARTING.start, slowest: [{ name: 'maps' }] } },
            },
            {
                type: 'event',
                topic: 'server.status',
                data: { ...STARTING, start: { ...STARTING.start, at: { file: 'server.cfg' } } },
            },
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

const PROCESS = {
    id: 7,
    runStarted: '2026-10-29T10:00:00Z',
    generation: 4,
    pid: 4120,
    parent: { pid: 3312, name: 'FXServer.exe' },
    image: 'C:\\Windows\\System32\\WindowsPowerShell\\v1.0\\powershell.exe',
    name: 'powershell.exe',
    commandLine: 'powershell -NoProfile -Command exit',
    truncated: false,
    kind: 'shell',
    startedAt: '2026-10-29T11:05:00Z',
    lastStartedAt: '2026-10-29T11:06:00Z',
    endedAt: null,
    count: 3,
};

const CRASH = {
    id: 2,
    at: '2026-10-29T14:02:07Z',
    kind: 'crash',
    runStarted: '2026-10-29T10:00:00Z',
    generation: 4,
    build: 35245,
    exit: 'exit code 0xFFFFFFFE (ended by its crash dump server after a crash)',
    exitCode: '0xFFFFFFFE',
    signal: null,
    hangCause: null,
    signature: '0xC0000005 citizen-scripting-lua.dll+0x1a2b3',
    analysisState: 'done',
    analysis: {
        exception: { code: '0xC0000005', name: 'EXCEPTION_ACCESS_VIOLATION_WRITE', address: '0x10' },
        thread: { id: 812, name: 'UV loop: svMain' },
        frames: [{ module: 'citizen-scripting-lua.dll', offset: '0x1a2b3', scanned: false }],
        module: 'citizen-scripting-lua.dll',
        threads: [],
        syscallsReadable: null,
    },
    watchdogStack: null,
    files: [{ name: '6c1f0b2e.dmp', bytes: 1_300_000, kind: 'minidump', kept: true }],
    group: { count: 3, firstAt: '2026-10-27T09:00:00Z' },
    updatedAt: '2026-10-29T14:02:08Z',
};

const DESTINATION = {
    protocol: 'tcp',
    address: '127.0.0.1',
    port: 3306,
    process: 'FXServer.exe',
    scope: 'loopback',
    service: 'mysql',
    open: 4,
    peak: 9,
    firstSeen: '2026-10-28T10:00:00Z',
    lastSeen: '2026-10-29T14:02:00Z',
};

const LISTENER = {
    protocol: 'udp',
    address: '0.0.0.0',
    port: 30120,
    process: 'FXServer.exe',
    role: 'game',
    inbound: 0,
};

const FOLDER = { files: 2, bytes: 515_000_000, maxFiles: 10, maxBytes: 21_474_836_480, free: 41_000_000_000 };

const ALERTS = {
    alerts: [
        {
            key: 'process:7',
            kind: 'shell',
            at: '2026-10-29T11:05:00Z',
            lastAt: '2026-10-29T11:06:00Z',
            count: 3,
            name: 'powershell.exe',
            parent: 'FXServer.exe',
            crash: null,
        },
        {
            key: 'crash:2',
            kind: 'hang',
            at: '2026-10-29T14:20:00Z',
            lastAt: '2026-10-29T14:20:00Z',
            count: 1,
            name: null,
            parent: null,
            crash: {
                exception: null,
                module: null,
                thread: 'UV loop: svMain',
                exitCode: null,
                signal: null,
                busy: false,
                waitingIn: 'futex',
                snapshot: true,
                groupCount: 1,
            },
        },
    ],
    since: '2026-10-28T14:20:00Z',
};

describe('the diagnostics topics', () => {
    it('should accept their snapshots and events', () => {
        for (const message of [
            { type: 'snapshot', topic: 'server.alerts', data: ALERTS },
            { type: 'event', topic: 'server.alerts', data: { alerts: [], since: ALERTS.since } },
            {
                type: 'snapshot',
                topic: 'server.diagnostics',
                data: {
                    processes: [PROCESS],
                    destinations: [DESTINATION],
                    listeners: [LISTENER],
                    crashes: [CRASH],
                    crashFolder: FOLDER,
                },
            },
            { type: 'event', topic: 'server.diagnostics', data: { kind: 'process', process: PROCESS } },
            {
                type: 'event',
                topic: 'server.diagnostics',
                data: { kind: 'connections', destinations: [DESTINATION], listeners: [] },
            },
            { type: 'event', topic: 'server.diagnostics', data: { kind: 'crash', crash: CRASH } },
            { type: 'event', topic: 'server.diagnostics', data: { kind: 'crashFolder', folder: FOLDER } },
        ]) {
            expect(parse(message), JSON.stringify(message)).toEqual(message);
        }
    });

    it('should refuse a field out of the contract', () => {
        for (const message of [
            {
                type: 'snapshot',
                topic: 'server.alerts',
                data: { alerts: [{ ...ALERTS.alerts[0], kind: 'virus' }] },
            },
            {
                type: 'event',
                topic: 'server.diagnostics',
                data: { kind: 'process', process: { ...PROCESS, kind: 'backdoor' } },
            },
            {
                type: 'event',
                topic: 'server.diagnostics',
                data: {
                    kind: 'crash',
                    crash: { ...CRASH, files: [{ name: 'a.dmp', bytes: -1, kind: 'minidump', kept: true }] },
                },
            },
            {
                type: 'event',
                topic: 'server.diagnostics',
                data: {
                    kind: 'connections',
                    destinations: [{ ...DESTINATION, scope: 'nearby' }],
                    listeners: [],
                },
            },
            { type: 'event', topic: 'server.diagnostics', data: { kind: 'reboot' } },
            {
                type: 'snapshot',
                topic: 'server.diagnostics',
                data: { processes: [], destinations: [], listeners: [], crashes: [] },
            },
        ]) {
            expect(parse(message), JSON.stringify(message)).toBeNull();
        }
    });
});
