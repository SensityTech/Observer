import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import type { CfgCheck } from '../api/generated/CfgCheck';
import type { CfgFinding } from '../api/generated/CfgFinding';
import type { CfgReport } from '../api/generated/CfgReport';
import type { FolderCheck } from '../api/generated/FolderCheck';
import type { FolderReport } from '../api/generated/FolderReport';
import type { LaunchSettings } from '../api/generated/LaunchSettings';
import type { OneSync } from '../api/generated/OneSync';
import type { ServerStatus } from '../api/generated/ServerStatus';
import type { Severity } from '../api/generated/Severity';
import { DEFAULT_LAUNCH } from '../api/launch';
import { isRunning } from '../server/state';

export const SETUP_STEPS = ['folder', 'cfg', 'build', 'start'] as const;
export type SetupStep = (typeof SETUP_STEPS)[number];

/** A report with the request it answers: it says nothing of a path typed since. */
export type Checked<Request, Report> = { request: Request; report: Report };

export const cfgCheckOf = (draft: LaunchSettings): CfgCheck => ({
    dataPath: draft.dataPath.trim(),
    cfgFile: draft.cfgFile.trim(),
    onesync: draft.onesync,
});

/** The folder step goes on once the folder typed now was checked and FXServer can run from it. */
export const folderPasses = (checked: Checked<FolderCheck, FolderReport> | null, draft: LaunchSettings) =>
    checked !== null && checked.request.dataPath === draft.dataPath.trim() && checked.report.problem === null;

/**
 * The cfg step goes on once the cfg chosen now was checked and nothing in it keeps FXServer from
 * starting under Observer: the rule the save applies again.
 */
export const cfgPasses = (checked: Checked<CfgCheck, CfgReport> | null, draft: LaunchSettings) => {
    if (checked === null) {
        return false;
    }
    const { request, report } = checked;
    return (
        request.dataPath === draft.dataPath.trim() &&
        request.cfgFile === draft.cfgFile.trim() &&
        report.folder === null &&
        report.problem === null &&
        !report.findings.some((finding) => finding.severity === 'blocking')
    );
};

/**
 * The cfg once a folder is accepted: the one Observer detected there, in place of `taken`, the cfg
 * detected in a folder checked before, and of a default the folder does not hold. A cfg the owner
 * chose stays.
 */
export const cfgAfterFolder = (report: FolderReport, cfgFile: string, taken: string | null) =>
    report.detectedCfg !== null &&
    (cfgFile === taken || (cfgFile === DEFAULT_LAUNCH.cfgFile && !report.cfgFiles.includes(cfgFile)))
        ? report.detectedCfg
        : cfgFile;

/** The OneSync the start step offers until the owner picks one: the cfg's, else the saved one, else on. */
export const preselectedOnesync = (cfg: OneSync | null, saved: OneSync | null): OneSync =>
    cfg ?? saved ?? 'on';

/**
 * What the start step can do after saving: nothing without a build, else start FXServer, or
 * restart it while it runs. Builds not known yet leave the answer to the API.
 */
export const startAction = (
    builds: ArtifactsStatus | null,
    status: ServerStatus | null,
): 'start' | 'restart' | null => {
    if (builds !== null && builds.selected === null) {
        return null;
    }
    return status !== null && isRunning(status.state) ? 'restart' : 'start';
};

const SEVERITIES: Severity[] = ['blocking', 'warning', 'note'];

/** The findings by severity, the most serious first, each group in the order the scan met them. */
export const groupFindings = (findings: CfgFinding[]) =>
    SEVERITIES.map((severity) => ({
        severity,
        findings: findings.filter((finding) => finding.severity === severity),
    })).filter((group) => group.findings.length > 0);

export const LAUNCH_STEPS = ['started', 'port', 'licence', 'bridge', 'ready'] as const;
export type LaunchStep = (typeof LAUNCH_STEPS)[number];

/** How far the first launch went: the steps passed in order, the one that failed, if it still goes. */
export type Checklist = { passed: number; failed: LaunchStep | null; waiting: boolean };

/** The step a run of FXServer failed at, from what ended it. */
const failedAt = (status: ServerStatus): LaunchStep | null => {
    const cause = status.cause;
    if (cause?.kind === 'bindError') {
        return 'port';
    }
    if (cause?.kind === 'licenceRejected') {
        return 'licence';
    }
    if (cause?.kind === 'hang' && cause.cause === 'noHeartbeat') {
        return 'bridge';
    }
    const ended =
        status.state === 'crashed' ||
        status.state === 'crashLoop' ||
        cause?.kind === 'exited' ||
        cause?.kind === 'fatalError';
    return ended ? 'ready' : null;
};

/**
 * The first launch as `status` shows it, against the status from before it was asked for. Only
 * the run it started counts, the one of a later generation: the run being restarted is not it.
 * Ready needs the bridge's heartbeat and an HTTP answer, so `online` passes every step at once.
 */
export const checklist = (before: ServerStatus | null, status: ServerStatus): Checklist => {
    const changed = before === null || status.since !== before.since;
    if (changed && status.state === 'stopped' && status.cause?.kind === 'spawnFailed') {
        return { passed: 0, failed: 'started', waiting: false };
    }
    if (status.generation <= (before?.generation ?? 0)) {
        return { passed: 0, failed: null, waiting: true };
    }
    if (status.state === 'online' || status.state === 'degraded') {
        return { passed: LAUNCH_STEPS.length, failed: null, waiting: false };
    }
    const failed = failedAt(status);
    // A run being stopped is followed to its end: stopped, crashed, or the new run of a restart.
    const going = status.state === 'starting' || status.state === 'hung' || status.state === 'stopping';
    return { passed: 1, failed, waiting: failed === null && going };
};
