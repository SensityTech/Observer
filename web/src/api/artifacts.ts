import { queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { ApplyMode } from './generated/ApplyMode';
import type { ArtifactSettingsChange } from './generated/ArtifactSettingsChange';
import type { ArtifactSettingsField } from './generated/ArtifactSettingsField';
import type { ArtifactSettingsIssue } from './generated/ArtifactSettingsIssue';
import type { ArtifactSettingsProblem } from './generated/ArtifactSettingsProblem';
import type { ArtifactsStatus } from './generated/ArtifactsStatus';
import type { CatalogView } from './generated/CatalogView';
import type { CleanupPreview } from './generated/CleanupPreview';
import type { CleanupResult } from './generated/CleanupResult';
import type { DownloadProgress } from './generated/DownloadProgress';
import type { JobKind } from './generated/JobKind';
import type { JobStep } from './generated/JobStep';
import type { Protection } from './generated/Protection';
import type { SelectBuild } from './generated/SelectBuild';

const PATH = '/servers/default/artifacts';

// Every change answers the whole status, which the `server.artifacts` topic carries too.

/** The catalog of the host's platform, which Observer answers from memory while it is fresh. */
export const catalogQuery = queryOptions({
    queryKey: ['artifacts', 'catalog'],
    queryFn: ({ signal }) => getJson<CatalogView>(`${PATH}/catalog`, signal),
    staleTime: 60_000,
});

export const refreshCatalog = () => getJson<CatalogView>(`${PATH}/catalog?refresh=true`);

export const cleanupQuery = queryOptions({
    queryKey: ['artifacts', 'cleanup'],
    queryFn: ({ signal }) => getJson<CleanupPreview>(`${PATH}/cleanup`, signal),
    staleTime: 0,
});

export const installBuild = (build: number, allowEndOfLife: boolean) =>
    send<ArtifactsStatus>('POST', `${PATH}/install`, { build, allowEndOfLife });
export const importBuild = (folder: string, build: number) =>
    send<ArtifactsStatus>('POST', `${PATH}/import`, { folder, build });
export const cancelJob = () => send<ArtifactsStatus>('DELETE', `${PATH}/job`);
export const selectBuild = (request: SelectBuild) =>
    send<ArtifactsStatus>('PUT', `${PATH}/selection`, request);
export const cancelPending = () => send<ArtifactsStatus>('DELETE', `${PATH}/pending`);
export const deleteBuild = (build: number) => send<ArtifactsStatus>('DELETE', `${PATH}/builds/${build}`);
export const cleanUp = (builds: number[]) => send<CleanupResult>('POST', `${PATH}/cleanup`, { builds });
/** Refused with `conflict` when the settings changed since the version the change was made from. */
export const saveArtifactSettings = (change: ArtifactSettingsChange) =>
    send<ArtifactsStatus>('PUT', `${PATH}/settings`, change);

/** The build below which FXServer kept its KVP data in another format. */
export const KVP_BUILD = 6449;

/** The steps a job of this kind goes through, in order. */
export const stepsOf = (kind: JobKind): JobStep[] =>
    kind === 'install'
        ? ['preCheck', 'download', 'verify', 'extract', 'validate', 'commit']
        : ['preCheck', 'copy', 'validate', 'commit'];

/** The seconds a download has left at its rate, or null before there is one. */
export const secondsLeft = (download: DownloadProgress) =>
    download.rate > 0 ? Math.ceil(Math.max(0, download.total - download.received) / download.rate) : null;

/** Why a build may not be deleted, as the server works it out. */
export const protectionsOf = (status: ArtifactsStatus, build: number): Protection[] =>
    (
        [
            ['selected', status.selected],
            ['pending', status.pending?.build ?? null],
            ['running', status.running],
            ['lastHealthy', status.lastHealthy?.build ?? null],
            ['probation', status.probation?.build ?? null],
        ] satisfies [Protection, number | null][]
    )
        .filter(([, protectedBuild]) => protectedBuild === build)
        .map(([reason]) => reason);

/**
 * Going back this far after a newer build ran needs the number typed. At the scheduled restart the
 * selected build counts as run, as FXServer runs it first when it starts before then.
 */
export const needsKvpConfirmation = (status: ArtifactsStatus, build: number, mode: ApplyMode) =>
    build < KVP_BUILD &&
    Math.max(status.highestRun ?? 0, mode === 'scheduledRestart' ? (status.selected ?? 0) : 0) >= KVP_BUILD;

/**
 * How the switch dialog applies a build unless the user picks otherwise: at the scheduled restart
 * when FXServer runs and one is ahead, else at the next start; never a restart nobody chose.
 */
export const defaultMode = (running: boolean, restartAhead: boolean): ApplyMode =>
    running && restartAhead ? 'scheduledRestart' : 'nextStart';

const FIELDS = { keepBuilds: true, branches: true } satisfies Record<ArtifactSettingsField, true>;

const PROBLEMS = {
    outOfRange: true,
    tooMany: true,
    invalid: true,
    master: true,
    duplicate: true,
} satisfies Record<ArtifactSettingsProblem, true>;

const isIssue = (value: unknown): value is ArtifactSettingsIssue =>
    typeof value === 'object' &&
    value !== null &&
    'field' in value &&
    typeof value.field === 'string' &&
    Object.hasOwn(FIELDS, value.field) &&
    'problem' in value &&
    typeof value.problem === 'string' &&
    Object.hasOwn(PROBLEMS, value.problem) &&
    'index' in value &&
    (value.index === null || Number.isSafeInteger(value.index));

/** The issues an `invalid_artifact_settings` refusal lists, leaving out any the form cannot place. */
export const settingsIssues = (params: Record<string, unknown>): ArtifactSettingsIssue[] =>
    Array.isArray(params.issues) ? params.issues.filter(isIssue) : [];
