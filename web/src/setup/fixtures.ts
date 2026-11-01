// What the tests of the server setup start from: a server folder on D:, its server.cfg that execs
// a resources cfg, and the findings a cfg kept from txAdmin's days typically gives.

import type { CfgFinding } from '../api/generated/CfgFinding';
import type { CfgReport } from '../api/generated/CfgReport';
import type { FolderReport } from '../api/generated/FolderReport';

export const DATA_PATH = 'D:\\FXServer\\server-data';

export const FOLDER: FolderReport = {
    problem: null,
    pathProblem: null,
    suggestion: null,
    resources: { resources: 54, categories: 6, withoutManifest: ['old-hud'], truncated: false },
    cfgFiles: ['server.cfg', 'configs/live.cfg'],
    detectedCfg: 'server.cfg',
};

export const finding = (
    kind: CfgFinding['kind'],
    severity: CfgFinding['severity'],
    line: number | null,
    params: CfgFinding['params'] = {},
    file = 'server.cfg',
): CfgFinding => ({ kind, severity, file, line, params });

export const CFG: CfgReport = {
    folder: null,
    problem: null,
    pathProblem: null,
    files: [
        { path: 'server.cfg', state: 'read' },
        { path: 'resources.cfg', state: 'read' },
        { path: '@chat/extra.cfg', state: 'resource' },
    ],
    facts: {
        game: null,
        endpoints: [
            { protocol: 'tcp', address: '0.0.0.0:30140' },
            { protocol: 'udp', address: '0.0.0.0:30140' },
        ],
        maxClients: 48,
        onesync: null,
        hostname: '^2Lab ^7server',
        licence: 'set',
        gameBuild: null,
    },
    findings: [
        finding('editedByTxAdmin', 'note', 12, { count: 2 }),
        finding('startsMonitor', 'warning', 9, { name: 'monitor' }),
        finding('resourceMissing', 'warning', 4, { name: 'esx_menu' }, 'resources.cfg'),
        finding('execResource', 'note', 14, { target: '@chat/extra.cfg' }),
    ],
};
