import { defaultParseSearch, defaultStringifySearch } from '@tanstack/react-router';
import { describe, expect, it } from 'vitest';

import { FIRST_PAGE } from '../api/entities';
import { filtered, listingOf, parseEntitiesSearch, withFilters, withPage, withSort } from './search';

/** The address as the router reads it, then as the page reads that. */
const read = (address: string) => parseEntitiesSearch(defaultParseSearch(address));

describe('parseEntitiesSearch', () => {
    it('should read every filter the address holds, the sort and the page', () => {
        expect(
            read(
                '?kind=vehicle&class=car&resource=qb-garages&owner=12&bucket=0&search=adder&sort=owner&descending=true&page=3',
            ),
        ).toEqual({
            kind: 'vehicle',
            class: 'car',
            resource: 'qb-garages',
            owner: 12,
            bucket: 0,
            search: 'adder',
            sort: 'owner',
            descending: true,
            page: 3,
        });
    });

    it('should leave out what it does not know and what the list opens on', () => {
        expect(
            read('?kind=plane&class=tank&owner=0&bucket=-1&search=%20%20&sort=model&descending=no&page=1'),
        ).toEqual({});
        expect(read('?owner=nobody&page=two&sort=size')).toEqual({});
    });

    it('should drop a class under a kind that has none', () => {
        expect(read('?kind=ped&class=car')).toEqual({ kind: 'ped' });
        expect(read('?class=train')).toEqual({ class: 'train' });
    });

    it('should keep an empty resource, which asks for the entities no resource created', () => {
        expect(read('?resource=')).toEqual({ resource: '' });
        expect(read(defaultStringifySearch({ resource: '' }))).toEqual({ resource: '' });
        expect(listingOf(read('?resource=')).resource).toBe('');
    });

    it('should keep a resource or a search made of digits as text', () => {
        expect(read('?resource=911&search=1885233650')).toEqual({ resource: '911', search: '1885233650' });
    });

    it('should cut a search at what the API takes', () => {
        expect(read(`?search=${'a'.repeat(80)}`).search).toHaveLength(64);
    });
});

describe('listingOf', () => {
    it('should ask for the list as it opens when the address says nothing', () => {
        expect(listingOf({})).toEqual(FIRST_PAGE);
    });

    it('should count pages from 0, as the API does', () => {
        expect(listingOf({ kind: 'object', owner: 'none', page: 3 })).toEqual({
            ...FIRST_PAGE,
            kinds: ['object'],
            owner: 'none',
            page: 2,
        });
    });
});

describe('filtered', () => {
    it('should not take a sort or a page for a filter', () => {
        expect(filtered({ sort: 'owner', descending: true, page: 2 })).toBe(false);
        expect(filtered({ resource: '' })).toBe(true);
        expect(filtered({ bucket: 0 })).toBe(true);
    });
});

describe('withFilters', () => {
    it('should start the list over and keep its sort', () => {
        expect(
            withFilters({ kind: 'ped', sort: 'bucket', descending: true, page: 4 }, { kind: 'vehicle' }),
        ).toEqual({ kind: 'vehicle', sort: 'bucket', descending: true });
    });
});

describe('withSort', () => {
    it('should turn the sort round on its own column, and start from the smallest on another', () => {
        expect(withSort({ page: 2 }, 'model')).toEqual({ descending: true });
        expect(withSort({ descending: true }, 'model')).toEqual({});
        expect(withSort({ sort: 'owner', descending: true, kind: 'ped' }, 'bucket')).toEqual({
            kind: 'ped',
            sort: 'bucket',
        });
    });
});

describe('withPage', () => {
    it('should leave the first page out of the address', () => {
        expect(withPage({ kind: 'ped', page: 3 }, 1)).toEqual({ kind: 'ped' });
        expect(withPage({ kind: 'ped' }, 2)).toEqual({ kind: 'ped', page: 2 });
    });
});
