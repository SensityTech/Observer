import { queryOptions } from '@tanstack/react-query';

import { send } from './client';
import type { CfgCheck } from './generated/CfgCheck';
import type { CfgReport } from './generated/CfgReport';
import type { CfgText } from './generated/CfgText';
import type { CfgTextQuery } from './generated/CfgTextQuery';
import type { Discovery } from './generated/Discovery';
import type { FileListing } from './generated/FileListing';
import type { FilesQuery } from './generated/FilesQuery';
import type { FolderCheck } from './generated/FolderCheck';
import type { FolderListing } from './generated/FolderListing';
import type { FolderQuery } from './generated/FolderQuery';
import type { FolderReport } from './generated/FolderReport';
import type { LaunchChange } from './generated/LaunchChange';
import type { StoredLaunch } from './generated/StoredLaunch';

const PATH = '/servers/default/setup';

// The checks change nothing: they are POSTs because a path never goes in an address.
export const checkFolder = (check: FolderCheck) => send<FolderReport>('POST', `${PATH}/folder`, check);
export const checkCfg = (check: CfgCheck) => send<CfgReport>('POST', `${PATH}/cfg`, check);
export const listFolders = (query: FolderQuery) => send<FolderListing>('POST', `${PATH}/folders`, query);

/** The servers found on the host, searched once a session: Rescan searches again. */
export const discoveryQuery = queryOptions({
    queryKey: ['servers', 'default', 'setup', 'discovery'],
    queryFn: () => send<Discovery>('POST', `${PATH}/discover`),
    staleTime: Infinity,
});

export const folderQuery = (query: FolderQuery) =>
    queryOptions({
        queryKey: ['servers', 'default', 'setup', 'folders', query.path, query.prefix],
        queryFn: () => listFolders(query),
        staleTime: 10_000,
    });

/** One folder of the server folder, for the setup's file browser. */
export const filesQuery = (query: FilesQuery) =>
    queryOptions({
        queryKey: ['servers', 'default', 'setup', 'files', query.dataPath, query.path],
        queryFn: () => send<FileListing>('POST', `${PATH}/files`, query),
        staleTime: 10_000,
    });

/** A cfg as the setup shows it, its secrets left out by Observer. */
export const cfgTextQuery = (query: CfgTextQuery) =>
    queryOptions({
        queryKey: ['servers', 'default', 'setup', 'cfg-text', query.dataPath, query.cfgFile],
        queryFn: () => send<CfgText>('POST', `${PATH}/cfg/text`, query),
        staleTime: 5_000,
    });

export const folderCheckQuery = (dataPath: string) =>
    queryOptions({
        queryKey: ['servers', 'default', 'setup', 'folder', dataPath],
        queryFn: () => checkFolder({ dataPath }),
        staleTime: 10_000,
    });

/**
 * Checks the folder and the cfg again, whatever the reports said, then saves as the launch settings
 * do: refused with `setup_blocked` when something blocks, with `conflict` on a stale version.
 */
export const saveSetup = (change: LaunchChange) => send<StoredLaunch>('PUT', PATH, change);
