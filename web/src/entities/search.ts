import { FIRST_PAGE, type EntityFilters, type EntityListing } from '../api/entities';
import type { EntityKind } from '../api/generated/EntityKind';
import type { EntitySort } from '../api/generated/EntitySort';
import type { VehicleClass } from '../api/generated/VehicleClass';
import { defined, oneOf, text } from '../lib/search';

// The page keeps its filters, its sort and its page in the address, so that the map and the audit
// log can link to a filtered list: `/entities?resource=qb-garages`.

export const KINDS: EntityKind[] = ['vehicle', 'ped', 'object'];

export const CLASSES: VehicleClass[] = [
    'car',
    'bike',
    'boat',
    'helicopter',
    'plane',
    'submarine',
    'trailer',
    'train',
    'other',
];

export const SORTS: EntitySort[] = ['kind', 'model', 'resource', 'owner', 'bucket', 'position', 'netId'];

/** The longest search the API takes, in characters. */
export const MAX_SEARCH = 64;
const MAX_NET_ID = 65_535;
const MAX_BUCKET = 2_147_483_647;

/** The page's address; a field left out is the list as it opens. */
export type EntitiesSearch = {
    kind?: EntityKind;
    class?: VehicleClass;
    /** A resource's name, or an empty one for the entities no resource created. */
    resource?: string;
    /** A number is the net id of the player whose game simulates the entities. */
    owner?: 'none' | 'owned' | number;
    bucket?: number;
    search?: string;
    sort?: EntitySort;
    descending?: true;
    /** Counted from 1, as the page writes it. */
    page?: number;
};

const FILTERS = ['kind', 'class', 'resource', 'owner', 'bucket', 'search'] as const;

/** A whole number from `least` to `most`, as the address or a field gives it; anything else is left out. */
const whole = (value: unknown, least: number, most: number) => {
    const number = typeof value === 'string' && /^\d+$/.test(value.trim()) ? Number(value) : value;
    return typeof number === 'number' && Number.isSafeInteger(number) && number >= least && number <= most
        ? number
        : undefined;
};

const ownerOf = (value: unknown) =>
    value === 'none' || value === 'owned' ? value : whole(value, 1, MAX_NET_ID);

/**
 * The page's address as the page reads it, whatever was typed into it: unknown or malformed fields
 * are left out rather than refused, and a class is dropped under a kind that has none.
 */
export const parseEntitiesSearch = (raw: Record<string, unknown>): EntitiesSearch => {
    const kind = oneOf(KINDS, raw.kind);
    const sort = oneOf(SORTS, raw.sort);
    const search = text(raw.search);
    return defined({
        kind,
        class: kind === undefined || kind === 'vehicle' ? oneOf(CLASSES, raw.class) : undefined,
        // An empty name is a filter of its own, which a text filter would leave out.
        resource: raw.resource === '' ? '' : text(raw.resource),
        owner: ownerOf(raw.owner),
        bucket: whole(raw.bucket, 0, MAX_BUCKET),
        search: search === undefined ? undefined : [...search].slice(0, MAX_SEARCH).join(''),
        sort: sort === FIRST_PAGE.sort ? undefined : sort,
        descending: raw.descending === true || raw.descending === 'true' ? true : undefined,
        page: whole(raw.page, 2, Number.MAX_SAFE_INTEGER),
    });
};

/** What the address asks the API for. */
export const listingOf = (search: EntitiesSearch): EntityListing => ({
    kinds: search.kind === undefined ? [] : [search.kind],
    class: search.class ?? null,
    resource: search.resource ?? null,
    owner: search.owner ?? 'any',
    bucket: search.bucket ?? null,
    search: search.search ?? '',
    sort: search.sort ?? FIRST_PAGE.sort,
    descending: search.descending === true,
    page: (search.page ?? 1) - 1,
});

/** Whether the list is narrowed by anything. */
export const filtered = (search: EntitiesSearch) => FILTERS.some((name) => search[name] !== undefined);

/** The address with its filters replaced by `next`: the sort is kept, the list starts over. */
export const withFilters = (
    search: EntitiesSearch,
    next: Pick<EntitiesSearch, (typeof FILTERS)[number]>,
): EntitiesSearch => parseEntitiesSearch({ ...next, sort: search.sort, descending: search.descending });

/**
 * The address once a column's heading is pressed: the other way round for the column the list is
 * sorted by, from its smallest for another one. The list starts over.
 */
export const withSort = (search: EntitiesSearch, sort: EntitySort): EntitiesSearch => {
    const current = search.sort ?? FIRST_PAGE.sort;
    const { page: _page, ...kept } = search;
    return parseEntitiesSearch({
        ...kept,
        sort,
        descending: current === sort && search.descending !== true ? true : undefined,
    });
};

/** The address of a page of the same list, counted from 1. */
export const withPage = (search: EntitiesSearch, page: number): EntitiesSearch =>
    parseEntitiesSearch({ ...search, page });

/** The filters alone, as the map's layer takes them. */
export const filtersOf = (search: EntitiesSearch): EntityFilters => {
    const { sort: _sort, descending: _descending, page: _page, ...filters } = listingOf(search);
    return filters;
};
