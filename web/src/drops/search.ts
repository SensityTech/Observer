import type { ChartCategory } from '../api/generated/ChartCategory';
import type { DropCategory } from '../api/generated/DropCategory';
import type { DropKind } from '../api/generated/DropKind';
import type { DropsRange } from '../api/generated/DropsRange';

// The page keeps its range, its chart and the log's filters in the address, so the profile, the
// Diagnostics page and the cards of the page itself link to a filtered view: `/drops?run=12`.

export const RANGES: DropsRange[] = ['24h', '7d', '30d', '90d'];

/** The range the page opens on, and the one the API takes when none is named. */
export const DEFAULT_RANGE: DropsRange = '7d';

export const CHART_CATEGORIES: ChartCategory[] = ['unexpected', 'crash', 'timeout', 'kick', 'security'];

export const CATEGORIES: DropCategory[] = [
    'crash',
    'timeout',
    'security',
    'unknown',
    'serverCrash',
    'kick',
    'quit',
    'serverStop',
];

/** Each category's kinds, in the order the filter lists them. A crash has its signature instead. */
export const KINDS: Record<DropCategory, DropKind[]> = {
    quit: ['exit', 'disconnect', 'otherServer', 'reconnect', 'editor', 'reload', 'quitMessage', 'other'],
    kick: ['resource'],
    timeout: ['connection', 'pendingCommands', 'missedFrames', 'replaced'],
    crash: [],
    security: [
        'stateBagRate',
        'netEventRate',
        'latentNetEventRate',
        'commandRate',
        'netGameEventRate',
        'serverInitiated',
    ],
    serverStop: ['stop', 'restart', 'scheduled', 'shutdown', 'crashLoop', 'console'],
    serverCrash: ['crash', 'hang'],
    unknown: ['lost', 'missed', 'other'],
};

const ALL_KINDS = new Set<string>(Object.values(KINDS).flat());

/** What the log is narrowed to; every field left out matches everything. */
export type LogFilters = {
    range: DropsRange;
    category?: DropCategory;
    kind?: DropKind;
    resource?: string;
    /** A crash signature's id, as the signatures card lists it. */
    signature?: number;
    run?: number;
    /** The start of a name, a licence, or any identifier with its prefix. */
    player?: string;
    /** One player's profile. */
    profile?: number;
    /** A player and the licences strongly linked to theirs. */
    group?: number;
};

/** The page's address: its range and chart, and the log's filters. */
export type DropsSearch = Omit<LogFilters, 'range'> & {
    range?: DropsRange;
    chart?: ChartCategory;
};

/** The filters of the log the address may hold, in the order the query string writes them. */
const FILTERS = ['category', 'kind', 'resource', 'signature', 'run', 'player', 'profile', 'group'] as const;

const oneOf = <T extends string>(choices: readonly T[], value: unknown): T | undefined =>
    choices.find((choice) => choice === value);

/** A positive whole number, as the address or a form gives it; anything else is left out. */
const id = (value: unknown) => {
    const number = typeof value === 'string' && /^\d+$/.test(value.trim()) ? Number(value) : value;
    return typeof number === 'number' && Number.isSafeInteger(number) && number > 0 ? number : undefined;
};

/**
 * A text filter, trimmed; an empty one is left out. What the address parsed as a number or a
 * boolean, a licence's digits or a name such as `true`, comes back as the text it was.
 */
const text = (value: unknown) => {
    const written = typeof value === 'number' || typeof value === 'boolean' ? String(value) : value;
    return typeof written === 'string' && written.trim() !== '' ? written.trim() : undefined;
};

/** Drops the fields that are not set, so that two equal searches read the same. */
const defined = <T extends object>(object: T): T =>
    Object.fromEntries(Object.entries(object).filter(([, value]) => value !== undefined)) as T;

/**
 * The page's address as the page reads it, whatever was typed into it: unknown or malformed fields
 * are left out rather than refused, and a kind that is not one of its category's is dropped.
 */
export const parseDropsSearch = (raw: Record<string, unknown>): DropsSearch => {
    const category = oneOf(CATEGORIES, raw.category);
    const kind = oneOf([...ALL_KINDS] as DropKind[], raw.kind);
    return defined({
        range: oneOf(RANGES, raw.range),
        chart: oneOf(CHART_CATEGORIES, raw.chart),
        category,
        kind:
            kind !== undefined && (category === undefined || KINDS[category].includes(kind))
                ? kind
                : undefined,
        resource: text(raw.resource),
        signature: id(raw.signature),
        run: id(raw.run),
        player: text(raw.player),
        profile: id(raw.profile),
        group: id(raw.group),
    });
};

/** The log's filters out of the page's address, under the page's range. */
export const logFilters = (search: DropsSearch): LogFilters => ({
    ...Object.fromEntries(
        FILTERS.flatMap((name) => (search[name] === undefined ? [] : [[name, search[name]]])),
    ),
    range: search.range ?? DEFAULT_RANGE,
});

/** Whether the log is narrowed by anything besides the range. */
export const filtered = (search: DropsSearch) => FILTERS.some((name) => search[name] !== undefined);

/** The address with the log's filters replaced by `next`, the range and the chart kept. */
export const withFilters = (search: DropsSearch, next: Omit<DropsSearch, 'range' | 'chart'>): DropsSearch =>
    defined({ range: search.range, chart: search.chart, ...parseDropsSearch(next) });

/** The log's query string: the range, then each filter set, and the page to start after. */
export const logSearch = (filters: LogFilters, before?: string) => {
    const query = new URLSearchParams({ range: filters.range });
    for (const name of FILTERS) {
        const value = filters[name];
        if (value !== undefined) {
            query.set(name, String(value));
        }
    }
    if (before !== undefined) {
        query.set('before', before);
    }
    return `?${query.toString()}`;
};

/** The chart that shows a category of the breakdown, if any does: the expected ones have none. */
export const chartOf = (category: DropCategory | undefined): ChartCategory | undefined => {
    switch (category) {
        case 'crash':
        case 'timeout':
        case 'kick':
        case 'security':
            return category;
        case 'unknown':
            return 'unexpected';
        default:
            return undefined;
    }
};

/**
 * The address once a category of the breakdown is picked, or `undefined` to drop it: the log is
 * narrowed to it, and the chart follows a category it can show and keeps its own otherwise.
 */
export const withCategory = (search: DropsSearch, category: DropCategory | undefined): DropsSearch => {
    const { range, chart, ...filters } = search;
    return defined({
        ...withFilters({ range, chart }, { ...filters, category }),
        chart: chartOf(category) === undefined ? chart : undefined,
    });
};

/** The chart the page shows: the one chosen, else the one of the category the log is narrowed to. */
export const chartShown = (search: DropsSearch): ChartCategory =>
    search.chart ?? chartOf(search.category) ?? 'unexpected';

const DAY = 86_400_000;

/** The shortest range that still holds `instant`, for a link to the drops of a moment. */
export const rangeCovering = (instant: string, now: number): DropsRange => {
    const age = now - Date.parse(instant);
    if (age <= DAY) {
        return '24h';
    }
    if (age <= 7 * DAY) {
        return '7d';
    }
    return age <= 30 * DAY ? '30d' : '90d';
};
