import { queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { EntitiesSettings } from './generated/EntitiesSettings';
import type { EntityDeleteRequest } from './generated/EntityDeleteRequest';
import type { EntityDeletion } from './generated/EntityDeletion';
import type { EntityKind } from './generated/EntityKind';
import type { EntityMap } from './generated/EntityMap';
import type { EntityPage } from './generated/EntityPage';
import type { EntitySort } from './generated/EntitySort';
import type { MapBounds } from './generated/MapBounds';
import type { VehicleClass } from './generated/VehicleClass';

const PATH = '/servers/default/entities';

/** Every query about the list: what a sweep or a deletion changes. */
export const allEntities = { queryKey: ['entities', 'list'] };

/** Which entities a list or the map shows. */
export type EntityFilters = {
    /** None for every kind. */
    kinds: EntityKind[];
    class: VehicleClass | null;
    /** A resource's name, '' for the entities no resource created, null for any. */
    resource: string | null;
    /** A number is the net id of the player whose game simulates the entity. */
    owner: 'any' | 'none' | 'owned' | number;
    bucket: number | null;
    search: string;
};

export const NO_FILTERS: EntityFilters = {
    kinds: [],
    class: null,
    resource: null,
    owner: 'any',
    bucket: null,
    search: '',
};

export type EntityListing = EntityFilters & { sort: EntitySort; descending: boolean; page: number };

export const FIRST_PAGE: EntityListing = { ...NO_FILTERS, sort: 'model', descending: false, page: 0 };

/** The filters as the API reads them, each left out while it filters nothing. */
const filterParams = (filters: EntityFilters) => {
    const query = new URLSearchParams();
    if (filters.kinds.length > 0) {
        query.set('kinds', filters.kinds.join(','));
    }
    if (filters.class !== null) {
        query.set('class', filters.class);
    }
    if (filters.resource !== null) {
        query.set('resource', filters.resource);
    }
    if (filters.owner !== 'any') {
        query.set('owner', String(filters.owner));
    }
    if (filters.bucket !== null) {
        query.set('bucket', String(filters.bucket));
    }
    if (filters.search.trim() !== '') {
        query.set('search', filters.search.trim());
    }
    return query;
};

const asSearch = (query: URLSearchParams) => {
    const text = query.toString();
    return text === '' ? '' : `?${text}`;
};

export const entitiesSearch = (listing: EntityListing) => {
    const query = filterParams(listing);
    if (listing.sort !== FIRST_PAGE.sort) {
        query.set('sort', listing.sort);
    }
    if (listing.descending) {
        query.set('descending', 'true');
    }
    if (listing.page > 0) {
        query.set('page', String(listing.page));
    }
    return asSearch(query);
};

export const entitiesQuery = (listing: EntityListing) =>
    queryOptions({
        queryKey: ['entities', 'list', 'page', listing],
        queryFn: ({ signal }) => getJson<EntityPage>(`${PATH}${entitiesSearch(listing)}`, signal),
    });

/** One entity as the last sweep saw it, by its net id alone. */
export const entityQuery = (netId: number) =>
    entitiesQuery({ ...FIRST_PAGE, sort: 'netId', search: `#${netId}` });

export const entitiesMapSearch = (filters: EntityFilters, cell: number, bounds: MapBounds | null) => {
    const query = filterParams(filters);
    query.set('cell', String(cell));
    if (bounds !== null) {
        for (const edge of ['west', 'east', 'south', 'north'] as const) {
            query.set(edge, String(bounds[edge]));
        }
    }
    return asSearch(query);
};

export const entitiesMapQuery = (filters: EntityFilters, cell: number, bounds: MapBounds | null) =>
    queryOptions({
        queryKey: ['entities', 'list', 'map', filters, cell, bounds],
        queryFn: ({ signal }) =>
            getJson<EntityMap>(`${PATH}/map${entitiesMapSearch(filters, cell, bounds)}`, signal),
    });

export const deleteEntities = (request: EntityDeleteRequest) =>
    send<EntityDeletion>('POST', `${PATH}/delete`, request);

export const entitiesSettingsQuery = queryOptions({
    queryKey: ['entities', 'settings'],
    queryFn: ({ signal }) => getJson<EntitiesSettings>(`${PATH}/settings`, signal),
});

export const saveEntitiesSettings = (settings: EntitiesSettings) =>
    send<EntitiesSettings>('PUT', `${PATH}/settings`, settings);

/** How often the server may list its entities, in seconds, as the API takes it. */
export const CADENCES = [5, 10, 15, 30, 60] as const;
