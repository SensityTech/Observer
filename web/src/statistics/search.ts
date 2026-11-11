import type { DropsRange } from '../api/generated/DropsRange';
import type { Range } from '../api/generated/Range';
import { oneOf } from '../lib/search';

// The page keeps its tab, its ranges, the zoom, the lanes added and the comparison in the address,
// so that a view is shared by its link. What the address bar holds is the user's to edit: what does
// not read is dropped.

export type Tab = 'timeline' | 'insights';

/** Lanes a viewer adds under the four the timeline always draws. */
export type ExtraLane = 'ping' | 'network' | 'heap' | 'disk';

export const EXTRA_LANES: readonly ExtraLane[] = ['ping', 'network', 'heap', 'disk'];
export const TIMELINE_RANGES: readonly Range[] = ['1h', '6h', '24h', '7d', '30d'];
export const INSIGHT_RANGES: readonly DropsRange[] = ['24h', '7d', '30d', '90d'];
export const DEFAULT_TIMELINE_RANGE: Range = '24h';
export const DEFAULT_INSIGHT_RANGE: DropsRange = '7d';

export type StatisticsSearch = {
    tab?: Tab;
    range?: Range;
    insights?: DropsRange;
    /** A zoomed window, Unix seconds. */
    from?: number;
    to?: number;
    lanes?: ExtraLane[];
    compare?: boolean;
};

const second = (value: unknown) =>
    typeof value === 'number' && Number.isSafeInteger(value) && value > 0 ? value : undefined;

export const parseStatisticsSearch = (search: Record<string, unknown>): StatisticsSearch => {
    const from = second(search.from);
    const to = second(search.to);
    const zoomed = from !== undefined && to !== undefined && to > from;
    const given = Array.isArray(search.lanes)
        ? search.lanes
        : typeof search.lanes === 'string'
          ? search.lanes.split(',')
          : [];
    const lanes = EXTRA_LANES.filter((lane) => given.includes(lane));
    return {
        tab: search.tab === 'insights' ? 'insights' : undefined,
        range: oneOf(TIMELINE_RANGES, search.range),
        insights: oneOf(INSIGHT_RANGES, search.insights),
        from: zoomed ? from : undefined,
        to: zoomed ? to : undefined,
        lanes: lanes.length > 0 ? lanes : undefined,
        compare: search.compare === true || search.compare === 'true' ? true : undefined,
    };
};

/** The lanes in their own order, or none for the address to leave out. */
export const lanesParam = (lanes: readonly ExtraLane[]) => {
    const kept = EXTRA_LANES.filter((lane) => lanes.includes(lane));
    return kept.length > 0 ? kept : undefined;
};

/** Seconds a fixed range covers. */
export const RANGE_SECONDS: Record<Range, number> = {
    '1h': 3600,
    '6h': 6 * 3600,
    '24h': 86_400,
    '7d': 7 * 86_400,
    '30d': 30 * 86_400,
};

/** The window the timeline reads: the zoom, or the range ending `now`. */
export const timelineWindow = (search: StatisticsSearch, now: number) => {
    if (search.from !== undefined && search.to !== undefined) {
        return { from: search.from, to: search.to, zoomed: true };
    }
    const range = search.range ?? DEFAULT_TIMELINE_RANGE;
    return { from: now - RANGE_SECONDS[range], to: now, zoomed: false };
};
