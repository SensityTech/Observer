import { queryOptions } from '@tanstack/react-query';

import { getJson } from './client';
import type { IntegrationDocs } from './generated/IntegrationDocs';

/**
 * The contract only changes with the build Observer runs. The server's side of it changes with
 * each start and with the launch settings: whoever shows it reads it again then.
 */
export const integrationDocsQuery = queryOptions({
    queryKey: ['docs', 'integration'],
    queryFn: ({ signal }) => getJson<IntegrationDocs>('/docs/integration', signal),
});
