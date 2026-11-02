import { queryOptions } from '@tanstack/react-query';

import { getJson, request, send } from './client';
import type { BackupCopy } from './generated/BackupCopy';
import type { BackupDestination } from './generated/BackupDestination';
import type { BackupRun } from './generated/BackupRun';
import type { BackupRunsPage } from './generated/BackupRunsPage';
import type { BackupSettingsChange } from './generated/BackupSettingsChange';
import type { BackupsView } from './generated/BackupsView';
import type { DestinationChange } from './generated/DestinationChange';
import type { DestinationDraft } from './generated/DestinationDraft';
import type { DestinationTarget } from './generated/DestinationTarget';
import type { DestinationTestRequest } from './generated/DestinationTestRequest';
import type { DestinationTestResult } from './generated/DestinationTestResult';
import type { SourceTestRequest } from './generated/SourceTestRequest';
import type { SourceTestResult } from './generated/SourceTestResult';

const PATH = '/servers/default/backups';
/** How often the page reads again while a backup runs, for its phase and its bytes. */
const RUNNING_EVERY_MS = 2000;

/** The settings, where the next backup reads from, the destinations and the latest runs. */
export const backupsQuery = queryOptions({
    queryKey: ['backups'],
    queryFn: ({ signal }) => getJson<BackupsView>(PATH, signal),
    refetchInterval: (query) => (query.state.data?.running == null ? false : RUNNING_EVERY_MS),
});

/** The latest `limit` runs, newest first, with their copies. */
export const backupRunsQuery = (limit: number) =>
    queryOptions({
        queryKey: ['backups', 'runs', limit],
        queryFn: ({ signal }) => getJson<BackupRunsPage>(`${PATH}/runs?limit=${limit}`, signal),
    });

/** Refused with `conflict` when the settings changed since the version the change was made from. */
export const saveBackupSettings = (change: BackupSettingsChange) =>
    send<BackupsView>('PUT', `${PATH}/settings`, change);

export const testBackupSource = (test: SourceTestRequest) =>
    send<SourceTestResult>('POST', `${PATH}/source/test`, test);

export const addDestination = (draft: DestinationDraft) =>
    send<BackupDestination>('POST', `${PATH}/destinations`, draft);

export const changeDestination = (id: number, change: DestinationChange) =>
    send<BackupDestination>('PUT', `${PATH}/destinations/${id}`, change);

export const deleteDestination = (id: number) => send('DELETE', `${PATH}/destinations/${id}`);

export const testDestination = (test: DestinationTestRequest) =>
    send<DestinationTestResult>('POST', `${PATH}/destinations/test`, test);

export const backUpNow = () => send<BackupRun>('POST', `${PATH}/run`);

export const cancelBackup = () => send('DELETE', `${PATH}/run`);

/** Hands a copy to the browser, as the console logs go: a plain link carries no session key. */
export const downloadCopy = async (copy: BackupCopy, name: string) => {
    const response = await request('GET', `${PATH}/copies/${copy.id}/download`);
    const url = URL.createObjectURL(await response.blob());
    try {
        const link = document.createElement('a');
        link.href = url;
        link.download = name;
        link.click();
    } finally {
        setTimeout(() => URL.revokeObjectURL(url), 0);
    }
};

/** How the page offers a destination: a local folder on a share is a kind of its own there. */
export type DestinationKind = 'local' | 'share' | 's3' | 'sftp';

/** A folder another machine serves: a UNC path, or a mount point of the usual places. */
export const isShare = (folder: string) =>
    folder.startsWith('\\\\') || folder.startsWith('//') || /^\/(mnt|media|net)\//.test(folder);

export const kindOf = (target: DestinationTarget): DestinationKind =>
    target.kind === 'local' ? (isShare(target.folder) ? 'share' : 'local') : target.kind;

/** Where a destination puts its copies, as one line: a folder, `host:port/folder`, `endpoint/bucket/prefix`. */
export const placeOf = (target: DestinationTarget) => {
    switch (target.kind) {
        case 'local':
            return target.folder;
        case 'sftp':
            return `${target.host}${target.port === 22 ? '' : `:${target.port}`}/${target.folder}`;
        case 's3':
            return `${target.endpoint.replace(/^https?:\/\//, '').replace(/\/$/, '')}/${target.bucket}/${target.prefix}`;
    }
};

/** The region a known store's address names, `auto` for Cloudflare R2, else AWS's default. */
export const regionOf = (endpoint: string) => {
    const host = endpoint.replace(/^https?:\/\//, '').split(/[/:]/)[0] ?? '';
    if (host.endsWith('.r2.cloudflarestorage.com')) {
        return 'auto';
    }
    const named = /^s3[.-]([a-z0-9-]+)\.(?:amazonaws\.com|backblazeb2\.com|wasabisys\.com)$/.exec(host);
    if (named?.[1] !== undefined && named[1] !== 'amazonaws') {
        return named[1];
    }
    const scaleway = /^s3\.([a-z]{2}-[a-z]{3})\.scw\.cloud$/.exec(host);
    return scaleway?.[1] ?? 'us-east-1';
};

/** AWS gives new buckets their own host; the stores compatible with it take the bucket in the path. */
export const pathStyleFor = (endpoint: string) => !/\.amazonaws\.com(?::\d+)?\/?$/.test(endpoint);

/** Amazon's address for a region, for an S3 destination left without one. */
export const awsEndpoint = (region: string) => `https://s3.${region}.amazonaws.com`;

/** The copies a run made that were stored, whether retention let some go since, and how many it tried. */
export const copiesOf = (run: BackupRun) => ({
    stored: run.copies.filter((copy) => copy.storedAt !== null).length,
    total: run.copies.length,
});

/** The copy a download reads: a stored one, on this host's own folders first, the quickest. */
export const downloadable = (run: BackupRun): BackupCopy | null =>
    run.copies
        .filter((copy) => copy.state === 'stored')
        .toSorted((a, b) => Number(b.kind === 'local') - Number(a.kind === 'local'))[0] ?? null;

/** How long a run took, in seconds, once it ended. */
export const tookSeconds = (run: BackupRun) =>
    run.finishedAt === null
        ? null
        : Math.max(0, (Date.parse(run.finishedAt) - Date.parse(run.startedAt)) / 1000);
