// What the diagnostics topics send, for the tests: a shell, a crash in a Lua module, a hang, and the
// tree's connections, shaped as the lab's server showed them.

import type { Alert } from '../api/generated/Alert';
import type { Crash } from '../api/generated/Crash';
import type { Destination } from '../api/generated/Destination';
import type { DiagnosticsSnapshot } from '../api/generated/DiagnosticsSnapshot';
import type { Listener } from '../api/generated/Listener';
import type { TreeProcess } from '../api/generated/TreeProcess';

export const SHELL: TreeProcess = {
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

export const CONHOST: TreeProcess = {
    ...SHELL,
    id: 1,
    pid: 3320,
    image: 'C:\\Windows\\System32\\conhost.exe',
    name: 'conhost.exe',
    commandLine: '\\??\\C:\\WINDOWS\\system32\\conhost.exe 0x4',
    kind: 'consoleHost',
    startedAt: '2026-10-29T10:00:01Z',
    lastStartedAt: '2026-10-29T10:00:01Z',
    count: 1,
};

export const CRASH: Crash = {
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
        frames: [
            { module: 'citizen-scripting-lua.dll', offset: '0x1a2b3', scanned: false },
            { module: null, offset: '0x7ff6a1b2c3d4', scanned: true },
        ],
        threads: [],
        syscallsReadable: null,
    },
    watchdogStack: null,
    files: [
        { name: '6c1f0b2e.dmp', bytes: 1_300_000, kind: 'minidump', kept: true },
        { name: '6c1f0b2e-full.dmp', bytes: 537_919_488, kind: 'fullDump', kept: true },
    ],
    group: { count: 3, firstAt: '2026-10-27T09:00:00Z' },
    updatedAt: '2026-10-29T14:02:08Z',
};

export const HANG: Crash = {
    ...CRASH,
    id: 3,
    at: '2026-10-29T14:20:00Z',
    kind: 'hang',
    exit: 'signal 9 (SIGKILL, killed)',
    exitCode: null,
    signal: 9,
    hangCause: 'noHeartbeat',
    signature: 'hang UV loop: svMain futex',
    analysis: {
        exception: null,
        thread: { id: 812, name: 'UV loop: svMain' },
        frames: [],
        threads: [
            {
                tid: 812,
                name: 'UV loop: svMain',
                state: 'sleeping',
                cpu: 0.4,
                syscall: 'futex',
                syscallGroup: 'lock',
            },
            { tid: 820, name: 'svNetwork', state: 'running', cpu: 12.5, syscall: null, syscallGroup: null },
        ],
        syscallsReadable: true,
    },
    watchdogStack: 'root',
    files: [{ name: 'hang-4-20261029T142000Z.json', bytes: 2_048, kind: 'hangReport', kept: true }],
    group: { count: 1, firstAt: '2026-10-29T14:20:00Z' },
};

export const MYSQL: Destination = {
    protocol: 'tcp',
    address: '127.0.0.1',
    port: 3306,
    process: 'FXServer.exe',
    scope: 'loopback',
    service: 'mysql',
    open: 4,
    peak: 9,
    firstSeen: '2026-10-20T10:00:00Z',
    lastSeen: '2026-10-29T14:02:00Z',
};

export const GAME: Listener = {
    protocol: 'tcp',
    address: '127.0.0.1',
    port: 30120,
    process: 'FXServer.exe',
    role: 'game',
    inbound: 12,
};

export const SNAPSHOT: DiagnosticsSnapshot = {
    processes: [SHELL, CONHOST],
    destinations: [MYSQL],
    listeners: [GAME],
    crashes: [HANG, CRASH],
    crashFolder: {
        files: 3,
        bytes: 539_221_536,
        maxFiles: 10,
        maxBytes: 21_474_836_480,
        free: 44_023_414_784,
    },
};

export const SHELL_ALERT: Alert = {
    key: 'process:7',
    kind: 'shell',
    at: SHELL.startedAt,
    lastAt: SHELL.lastStartedAt,
    count: 3,
    name: 'powershell.exe',
    parent: 'FXServer.exe',
    crash: null,
};

export const CRASH_ALERT: Alert = {
    key: 'crash:2',
    kind: 'crash',
    at: CRASH.at,
    lastAt: CRASH.at,
    count: 1,
    name: null,
    parent: null,
    crash: {
        exception: { code: '0xC0000005', name: 'EXCEPTION_ACCESS_VIOLATION_WRITE' },
        module: 'citizen-scripting-lua.dll',
        thread: 'UV loop: svMain',
        exit: null,
        busy: false,
        waitingIn: null,
        snapshot: false,
        groupCount: 3,
    },
};

export const HANG_ALERT: Alert = {
    key: 'crash:3',
    kind: 'hang',
    at: HANG.at,
    lastAt: HANG.at,
    count: 1,
    name: null,
    parent: null,
    crash: {
        exception: null,
        module: null,
        thread: 'UV loop: svMain',
        exit: null,
        busy: false,
        waitingIn: 'futex',
        snapshot: true,
        groupCount: 1,
    },
};
