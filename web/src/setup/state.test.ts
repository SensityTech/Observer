import { describe, expect, it } from 'vitest';

import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import type { LaunchSettings } from '../api/generated/LaunchSettings';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { DEFAULT_LAUNCH } from '../api/launch';
import { ARTIFACTS } from '../artifacts/fixtures';
import { CFG, DATA_PATH, FOLDER, finding } from './fixtures';
import {
    cfgAfterFolder,
    cfgCheckOf,
    cfgPasses,
    checklist,
    folderPasses,
    groupFindings,
    preselectedOnesync,
    startAction,
} from './state';

const draft = (change: Partial<LaunchSettings> = {}): LaunchSettings => ({
    ...DEFAULT_LAUNCH,
    dataPath: DATA_PATH,
    ...change,
});

const status = (overrides: Partial<ServerStatus>): ServerStatus => ({
    state: 'stopped',
    cause: null,
    since: '2026-10-30T14:00:00Z',
    startedAt: null,
    generation: 3,
    problems: [],
    build: null,
    players: null,
    restartAt: null,
    launch: 'ready',
    name: null,
    port: null,
    ...overrides,
});

const BUILDS = structuredClone(ARTIFACTS) as unknown as ArtifactsStatus;

/** The cfg check of `cfgFile` in the fixture folder, answered with `report`. */
const cfgChecked = (report = CFG, cfgFile = 'server.cfg') => ({
    request: { ...cfgCheckOf(draft()), cfgFile },
    report,
});

describe('folderPasses', () => {
    it('should go on only once the folder typed now was checked and accepted', () => {
        const checked = { request: { dataPath: DATA_PATH }, report: FOLDER };

        expect(folderPasses(null, draft())).toBe(false);
        expect(folderPasses(checked, draft())).toBe(true);
        expect(folderPasses(checked, draft({ dataPath: ` ${DATA_PATH} ` }))).toBe(true);
        expect(folderPasses(checked, draft({ dataPath: 'D:\\FXServer\\other' }))).toBe(false);
        expect(
            folderPasses(
                {
                    ...checked,
                    report: { ...FOLDER, problem: 'emptyResources', resources: null, cfgFiles: [] },
                },
                draft(),
            ),
        ).toBe(false);
    });
});

describe('cfgPasses', () => {
    it('should go on with warnings and notes, the save allowing them too', () => {
        expect(cfgPasses(cfgChecked(), draft())).toBe(true);
    });

    it('should stay while a finding blocks, the cfg cannot be read, or the folder is refused', () => {
        const blocking = { ...CFG, findings: [...CFG.findings, finding('noEndpoint', 'blocking', null)] };
        const unread = { ...CFG, problem: 'missing' as const, files: [], facts: null, findings: [] };
        const refused = { ...unread, problem: null, folder: 'noResources' as const };

        expect(cfgPasses(cfgChecked(blocking), draft())).toBe(false);
        expect(cfgPasses(cfgChecked(unread), draft())).toBe(false);
        expect(cfgPasses(cfgChecked(refused), draft())).toBe(false);
        expect(cfgPasses(null, draft())).toBe(false);
    });

    it('should stay once another cfg is typed, until it is checked', () => {
        expect(cfgPasses(cfgChecked(), draft({ cfgFile: 'configs/live.cfg' }))).toBe(false);
        expect(cfgPasses(cfgChecked(CFG, 'configs/live.cfg'), draft({ cfgFile: 'configs/live.cfg' }))).toBe(
            true,
        );
    });

    it('should read the severity of each finding, a noEndpoint left unchecked being a warning', () => {
        const unchecked = { ...CFG, findings: [finding('noEndpoint', 'warning', null)] };

        expect(cfgPasses(cfgChecked(unchecked), draft())).toBe(true);
    });
});

describe('cfgAfterFolder', () => {
    it('should take the detected cfg in place of a default the folder does not hold', () => {
        const live = { ...FOLDER, cfgFiles: ['configs/live.cfg'], detectedCfg: 'configs/live.cfg' };

        expect(cfgAfterFolder(live, 'server.cfg', null)).toBe('configs/live.cfg');
        expect(cfgAfterFolder(FOLDER, 'server.cfg', null)).toBe('server.cfg');
    });

    it('should replace the cfg detected in a folder checked before, which the owner did not choose', () => {
        expect(cfgAfterFolder(FOLDER, 'main.cfg', 'main.cfg')).toBe('server.cfg');
        expect(
            cfgAfterFolder({ ...FOLDER, cfgFiles: ['server.cfg', 'main.cfg'] }, 'main.cfg', 'main.cfg'),
        ).toBe('server.cfg');
    });

    it('should keep a cfg the owner chose, and the default when nothing was detected', () => {
        const live = { ...FOLDER, cfgFiles: ['configs/live.cfg'], detectedCfg: 'configs/live.cfg' };

        expect(cfgAfterFolder(live, 'deploy/prod/server.cfg', 'main.cfg')).toBe('deploy/prod/server.cfg');
        expect(cfgAfterFolder({ ...FOLDER, cfgFiles: [], detectedCfg: null }, 'server.cfg', null)).toBe(
            'server.cfg',
        );
    });
});

describe('preselectedOnesync', () => {
    it("should offer the cfg's OneSync, else the saved one, else on", () => {
        expect(preselectedOnesync('legacy', 'off')).toBe('legacy');
        expect(preselectedOnesync(null, 'off')).toBe('off');
        expect(preselectedOnesync(null, null)).toBe('on');
    });
});

describe('startAction', () => {
    it('should only save without a build, the build step having been skipped', () => {
        expect(startAction({ ...BUILDS, selected: null }, status({}))).toBeNull();
    });

    it('should start a stopped server, restart a running one, and leave unknown builds to the API', () => {
        expect(startAction(BUILDS, status({}))).toBe('start');
        expect(startAction(BUILDS, status({ state: 'crashed' }))).toBe('start');
        expect(startAction(BUILDS, status({ state: 'online' }))).toBe('restart');
        expect(startAction(null, null)).toBe('start');
    });
});

describe('groupFindings', () => {
    it('should put blocking first, then warnings, then notes, each in the order the scan met them', () => {
        const findings = [
            finding('editedByTxAdmin', 'note', 12, { count: 2 }),
            finding('startsMonitor', 'warning', 9, { name: 'monitor' }),
            finding('endpointOnPanel', 'blocking', 2, { protocol: 'tcp', address: '0.0.0.0:40125' }),
            finding('licenceMissing', 'warning', null),
        ];

        expect(groupFindings(findings)).toEqual([
            { severity: 'blocking', findings: [findings[2]] },
            { severity: 'warning', findings: [findings[1], findings[3]] },
            { severity: 'note', findings: [findings[0]] },
        ]);
    });

    it('should leave out the severities nothing was found for', () => {
        expect(groupFindings(CFG.findings).map((group) => group.severity)).toEqual(['warning', 'note']);
        expect(groupFindings([])).toEqual([]);
    });
});

describe('checklist', () => {
    const before = status({});

    it('should wait on the start while no new run has begun', () => {
        expect(checklist(before, before)).toEqual({ passed: 0, failed: null, waiting: true });
    });

    it('should pass the start once a new run starts, then wait on the port', () => {
        const starting = status({
            state: 'starting',
            generation: 4,
            cause: { kind: 'requested', by: 'ada' },
        });

        expect(checklist(before, starting)).toEqual({ passed: 1, failed: null, waiting: true });
    });

    it('should pass every step once FXServer is online', () => {
        const online = status({ state: 'online', generation: 4, cause: { kind: 'ready' } });

        expect(checklist(before, online)).toEqual({ passed: 5, failed: null, waiting: false });
    });

    it('should fail the step that the cause names', () => {
        const run = { generation: 4, since: '2026-10-30T14:00:05Z' };
        const cases = [
            [
                status({ ...run, state: 'crashed', cause: { kind: 'bindError', address: '0.0.0.0:30140' } }),
                'port',
            ],
            [
                status({ ...run, state: 'stopped', cause: { kind: 'licenceRejected', message: 'no' } }),
                'licence',
            ],
            [status({ ...run, state: 'hung', cause: { kind: 'hang', cause: 'noHeartbeat' } }), 'bridge'],
            // Ready needs an HTTP answer, which never came in the delay.
            [status({ ...run, state: 'hung', cause: { kind: 'hang', cause: 'noHttp' } }), 'ready'],
            [
                status({ ...run, state: 'crashed', cause: { kind: 'exited', description: 'exit code 1' } }),
                'ready',
            ],
            [status({ ...run, state: 'crashLoop', cause: { kind: 'crashLoop', failures: 3 } }), 'ready'],
        ] as const;

        for (const [failed, step] of cases) {
            expect(checklist(before, failed), step).toEqual({ passed: 1, failed: step, waiting: false });
        }
    });

    it('should fail the start when FXServer could not be started, not for an earlier failure', () => {
        const earlier = status({ cause: { kind: 'spawnFailed', reason: 'missingData' } });
        const again = { ...earlier, since: '2026-10-30T14:00:05Z' };

        expect(checklist(earlier, earlier)).toEqual({ passed: 0, failed: null, waiting: true });
        expect(checklist(earlier, again)).toEqual({ passed: 0, failed: 'started', waiting: false });
    });

    it('should not take the run being restarted for the new one', () => {
        const running = status({ state: 'online', generation: 3, cause: { kind: 'ready' } });
        const stopping = {
            ...running,
            state: 'stopping' as const,
            cause: { kind: 'restart' as const, by: 'ada' },
        };

        expect(checklist(running, stopping)).toEqual({ passed: 0, failed: null, waiting: true });
    });

    it('should keep waiting while the new run stops, for what it ends as', () => {
        const stopping = status({
            state: 'stopping',
            generation: 4,
            since: '2026-10-30T14:00:05Z',
            cause: { kind: 'requested', by: 'ada' },
        });

        expect(checklist(before, stopping)).toEqual({ passed: 1, failed: null, waiting: true });
    });

    it('should end without a failure when someone stops the new run', () => {
        const stopped = status({
            generation: 4,
            since: '2026-10-30T14:00:05Z',
            cause: { kind: 'requested', by: 'ada' },
        });

        expect(checklist(before, stopped)).toEqual({ passed: 1, failed: null, waiting: false });
    });
});
