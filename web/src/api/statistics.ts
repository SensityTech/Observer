import { queryOptions } from '@tanstack/react-query';

import { ApiError, getJson, send } from './client';
import type { DropsRange } from './generated/DropsRange';
import type { FrameDistribution } from './generated/FrameDistribution';
import type { LinkDismissal } from './generated/LinkDismissal';
import type { PlayerHours } from './generated/PlayerHours';
import type { RunMemory } from './generated/RunMemory';
import type { Series } from './generated/Series';
import type { SessionSurvival } from './generated/SessionSurvival';
import type { StabilityEffects } from './generated/StabilityEffects';
import type { Timeline } from './generated/Timeline';
import type { TimelineEvents } from './generated/TimelineEvents';

/** What the server answers when its owner turned the statistics off. */
export const FEATURE_DISABLED = 'feature_disabled';

export const isFeatureDisabled = (error: unknown) =>
    error instanceof ApiError && error.code === FEATURE_DISABLED;

/** A window that ends now is read again this often; one in the past never changes. */
const LIVE_EVERY = 30_000;
/** Insights move slowly: hours, days, runs. */
const INSIGHTS_EVERY = 5 * 60_000;

const isRecord = (value: unknown): value is Record<string, unknown> =>
    typeof value === 'object' && value !== null && !Array.isArray(value);
const isNumber = (value: unknown): value is number => typeof value === 'number' && Number.isFinite(value);
const isCount = (value: unknown) => isNumber(value) && value >= 0;
const isMaybeNumber = (value: unknown) => value === null || isNumber(value);
const numbers = (value: unknown, length: number) =>
    Array.isArray(value) && value.length === length && value.every(isMaybeNumber);

/**
 * The timeline's answer, checked where the chart divides and indexes by it: every series and band
 * as long as the grid.
 */
export const isTimeline = (value: unknown): value is Timeline => {
    if (!isRecord(value) || !isNumber(value.from) || !isNumber(value.step) || !isCount(value.buckets)) {
        return false;
    }
    const length = value.buckets as number;
    const frames = value.frames;
    const usual = value.usual;
    return (
        (value.step as number) > 0 &&
        Array.isArray(value.series) &&
        value.series.every(
            (data) => isRecord(data) && typeof data.series === 'string' && numbers(data.values, length),
        ) &&
        numbers(value.observed, length) &&
        numbers(value.online, length) &&
        (frames === null ||
            (isRecord(frames) &&
                numbers(frames.p50, length) &&
                numbers(frames.p95, length) &&
                numbers(frames.p99, length) &&
                numbers(frames.ticks, length) &&
                isMaybeNumber(frames.floor) &&
                isMaybeNumber(frames.top) &&
                isMaybeNumber(frames.since))) &&
        (usual === null || (isRecord(usual) && numbers(usual.low, length) && numbers(usual.high, length)))
    );
};

const checked = async <T>(path: string, signal: AbortSignal, valid: (value: unknown) => value is T) => {
    const body = await getJson<unknown>(path, signal);
    if (!valid(body)) {
        throw new Error(`${path} answered in an unexpected shape`);
    }
    return body;
};

/** A window of series in at most `points` buckets, Unix seconds. */
export const timelineQuery = (
    from: number,
    to: number,
    series: readonly Series[],
    points = 720,
    live = false,
) =>
    queryOptions({
        queryKey: ['metrics', 'timeline', from, to, series.join(','), points],
        queryFn: ({ signal }) =>
            checked(
                `/metrics/timeline?from=${from}&to=${to}&series=${series.join(',')}&points=${points}`,
                signal,
                isTimeline,
            ),
        refetchInterval: live ? LIVE_EVERY : false,
    });

export const eventsQuery = (from: number, to: number, live = false) =>
    queryOptions({
        queryKey: ['metrics', 'events', from, to],
        queryFn: ({ signal }) => getJson<TimelineEvents>(`/metrics/events?from=${from}&to=${to}`, signal),
        refetchInterval: live ? LIVE_EVERY : false,
    });

export const framesQuery = (range: DropsRange) =>
    queryOptions({
        queryKey: ['metrics', 'frames', range],
        queryFn: ({ signal }) => getJson<FrameDistribution>(`/metrics/frames?range=${range}`, signal),
        refetchInterval: INSIGHTS_EVERY,
    });

export const hoursQuery = queryOptions({
    queryKey: ['metrics', 'hours'],
    queryFn: ({ signal }) => getJson<PlayerHours>('/metrics/hours', signal),
    refetchInterval: INSIGHTS_EVERY,
});

export const runsQuery = (range: DropsRange) =>
    queryOptions({
        queryKey: ['metrics', 'runs', range],
        queryFn: ({ signal }) => getJson<RunMemory>(`/metrics/runs?range=${range}`, signal),
        refetchInterval: INSIGHTS_EVERY,
    });

export const sessionsQuery = (range: DropsRange) =>
    queryOptions({
        queryKey: ['drops', 'sessions', range],
        queryFn: ({ signal }) =>
            getJson<SessionSurvival>(`/servers/default/drops/sessions?range=${range}`, signal),
        refetchInterval: INSIGHTS_EVERY,
    });

export const effectsQuery = queryOptions({
    queryKey: ['drops', 'effects'],
    queryFn: ({ signal }) => getJson<StabilityEffects>('/servers/default/drops/effects', signal),
    refetchInterval: INSIGHTS_EVERY,
});

/** Says a link between a crash wave and a change is not related, or takes it back. */
export const dismissLink = (dismissal: LinkDismissal) => send('PUT', '/metrics/links', dismissal);
