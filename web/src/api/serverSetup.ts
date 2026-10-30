import { send } from './client';
import type { CfgCheck } from './generated/CfgCheck';
import type { CfgReport } from './generated/CfgReport';
import type { FolderCheck } from './generated/FolderCheck';
import type { FolderReport } from './generated/FolderReport';
import type { LaunchChange } from './generated/LaunchChange';
import type { StoredLaunch } from './generated/StoredLaunch';

const PATH = '/servers/default/setup';

// The checks change nothing: they are POSTs because a path never goes in an address.
export const checkFolder = (check: FolderCheck) => send<FolderReport>('POST', `${PATH}/folder`, check);
export const checkCfg = (check: CfgCheck) => send<CfgReport>('POST', `${PATH}/cfg`, check);

/**
 * Checks the folder and the cfg again, whatever the reports said, then saves as the Server page
 * does: refused with `setup_blocked` when something blocks, with `conflict` on a stale version.
 */
export const saveSetup = (change: LaunchChange) => send<StoredLaunch>('PUT', PATH, change);
