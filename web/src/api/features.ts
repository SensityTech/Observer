import { queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { Feature } from './generated/Feature';
import type { FeatureList } from './generated/FeatureList';
import type { FeatureSwitch } from './generated/FeatureSwitch';

/** Every optional feature, in a fixed order, with what it runs and whether it runs. */
export const featuresQuery = queryOptions({
    queryKey: ['features'],
    queryFn: ({ signal }) => getJson<FeatureList>('/features', signal),
});

/** Answered once the feature's tasks have stopped, which can take a while. */
export const switchFeature = (feature: Feature, enabled: boolean) =>
    send<FeatureList>('PUT', `/features/${feature}`, { enabled } satisfies FeatureSwitch);
