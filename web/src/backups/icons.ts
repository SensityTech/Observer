import { Cloud, FolderSymlink, HardDrive, Server } from 'lucide-react';

import type { DestinationKind } from '../api/backups';

/** What each kind of destination shows beside its name. */
export const KIND_ICONS = {
    local: HardDrive,
    share: FolderSymlink,
    s3: Cloud,
    sftp: Server,
} satisfies Record<DestinationKind, unknown>;
