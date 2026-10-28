import { queryOptions } from '@tanstack/react-query';

import { getJson } from './client';
import type { Health } from './generated/Health';

export const healthQuery = queryOptions({
    queryKey: ['health'],
    queryFn: ({ signal }) => getJson<Health>('/health', signal),
    refetchInterval: 10_000,
});
