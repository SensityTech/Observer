import { queryOptions } from '@tanstack/react-query';

import { getJson } from './client';
import type { About } from './generated/About';
import type { ThirdPartyLicences } from './generated/ThirdPartyLicences';

/** Both only change with the build Observer runs. */
export const aboutQuery = queryOptions({
    queryKey: ['about'],
    queryFn: ({ signal }) => getJson<About>('/about', signal),
    staleTime: Infinity,
});

export const licencesQuery = queryOptions({
    queryKey: ['about', 'licences'],
    queryFn: ({ signal }) => getJson<ThirdPartyLicences>('/about/licences', signal),
    staleTime: Infinity,
    retry: false,
});
