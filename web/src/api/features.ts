import { queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { Feature } from './generated/Feature';
import type { FeatureList } from './generated/FeatureList';
import type { FeatureSwitch } from './generated/FeatureSwitch';
import type { SftpListen } from './generated/SftpListen';
import type { SftpListenChange } from './generated/SftpListenChange';

/** Every optional feature, in a fixed order, with what it runs and whether it runs. */
export const featuresQuery = queryOptions({
    queryKey: ['features'],
    queryFn: ({ signal }) => getJson<FeatureList>('/features', signal),
});

/** Answered once the feature's tasks have stopped, which can take a while. */
export const switchFeature = (feature: Feature, enabled: boolean) =>
    send<FeatureList>('PUT', `/features/${feature}`, { enabled } satisfies FeatureSwitch);

/** Where SFTP listens, who says so, and what became of its port. */
export const sftpListenQuery = queryOptions({
    queryKey: ['features', 'sftp'],
    queryFn: ({ signal }) => getJson<SftpListen>('/settings/sftp', signal),
});

/** Answered once the port opened, was refused or closed. No address closes it. */
export const setSftpListen = (listen: string | null) =>
    send<SftpListen>('PUT', '/settings/sftp', { listen } satisfies SftpListenChange);
