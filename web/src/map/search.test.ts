import { defaultParseSearch } from '@tanstack/react-router';
import { describe, expect, it } from 'vitest';

import { parseMapSearch } from './search';

/** The address as the router reads it, then as the map reads that. */
const read = (address: string) => parseMapSearch(defaultParseSearch(address));

describe('parseMapSearch', () => {
    it('should read the entity the list sends the map to', () => {
        expect(read('?entity=1042')).toEqual({ entity: 1042 });
    });

    it('should leave out what is not a net id', () => {
        expect(read('')).toEqual({});
        expect(read('?entity=0')).toEqual({});
        expect(read('?entity=70000')).toEqual({});
        expect(read('?entity=car')).toEqual({});
        expect(read('?entity=1.5')).toEqual({});
    });
});
