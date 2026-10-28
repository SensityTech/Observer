import { queryOptions } from '@tanstack/react-query';

import { getJson } from './client';
import type { MetricsSeries } from './generated/MetricsSeries';
import type { Range } from './generated/Range';
import type { Series } from './generated/Series';

/** Live samples carry the chart between two reads; a read every five minutes settles any drift. */
export const REFETCH_EVERY = 5 * 60_000;

/** A range of series ending now, on the range's fixed grid of buckets. */
export const seriesQuery = (range: Range, series: readonly Series[]) =>
    queryOptions({
        queryKey: ['metrics', 'series', range, series.join(',')],
        queryFn: ({ signal }) =>
            getJson<MetricsSeries>(
                `/metrics/series?range=${encodeURIComponent(range)}&series=${series.join(',')}`,
                signal,
            ),
        refetchInterval: REFETCH_EVERY,
    });
