import { queryOptions } from '@tanstack/react-query';

import { getJson } from './client';
import type { Growth } from './generated/Growth';
import type { MetricsSeries } from './generated/MetricsSeries';
import type { Range } from './generated/Range';
import type { RunGrowth } from './generated/RunGrowth';
import type { Series } from './generated/Series';
import type { ThreadGroup } from './generated/ThreadGroup';
import type { ThreadShares } from './generated/ThreadShares';

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

const GROUPS = {
    main: true,
    network: true,
    sync: true,
    js: true,
    mono: true,
    other: true,
} satisfies Record<ThreadGroup, true>;

const isRecord = (value: unknown): value is Record<string, unknown> =>
    typeof value === 'object' && value !== null && !Array.isArray(value);
const isNumber = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value);

/** The answer of `/metrics/threads`, field by field: the card divides by what it reads. */
export const isThreadShares = (value: unknown): value is ThreadShares =>
    isRecord(value) &&
    isNumber(value.from) &&
    isNumber(value.to) &&
    isNumber(value.measured) &&
    (value.process === null || isNumber(value.process)) &&
    Array.isArray(value.threads) &&
    value.threads.every(
        (thread) =>
            isRecord(thread) &&
            typeof thread.name === 'string' &&
            typeof thread.group === 'string' &&
            Object.hasOwn(GROUPS, thread.group) &&
            isNumber(thread.cpu),
    );

const isGrowth = (value: unknown): value is Growth =>
    isRecord(value) &&
    isNumber(value.perHour) &&
    isNumber(value.fit) &&
    isNumber(value.start) &&
    isNumber(value.points) &&
    typeof value.leak === 'boolean';

/** The answer of `/metrics/growth`: `null` without a run under way. */
export const isRunGrowth = (value: unknown): value is RunGrowth | null =>
    value === null ||
    (isRecord(value) &&
        isNumber(value.startedAt) &&
        isNumber(value.from) &&
        isNumber(value.to) &&
        (value.private === null || isGrowth(value.private)) &&
        (value.handles === null || isGrowth(value.handles)));

const checked = async <T>(path: string, signal: AbortSignal, valid: (value: unknown) => value is T) => {
    const body = await getJson<unknown>(path, signal);
    if (!valid(body)) {
        throw new Error(`${path} answered in an unexpected shape`);
    }
    return body;
};

/** CPU per thread name of FXServer's main process over the last hour. */
export const threadsQuery = queryOptions({
    queryKey: ['metrics', 'threads'],
    queryFn: ({ signal }) => checked('/metrics/threads', signal, isThreadShares),
    refetchInterval: REFETCH_EVERY,
});

/** How FXServer's memory and handles grew over the run under way. */
export const growthQuery = queryOptions({
    queryKey: ['metrics', 'growth'],
    queryFn: ({ signal }) => checked('/metrics/growth', signal, isRunGrowth),
    refetchInterval: REFETCH_EVERY,
});
