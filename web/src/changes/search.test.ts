import { describe, expect, it } from 'vitest';

import { changesSearch, resourceSearch } from './search';

describe('the Changes addresses', () => {
    it('should keep start ids and the disk, and drop what the address bar holds besides', () => {
        expect(changesSearch({ from: 12, to: 'disk' })).toEqual({ from: 12, to: 'disk' });
        expect(changesSearch({ from: 'twelve', to: -3 })).toEqual({ from: undefined, to: undefined });
        expect(changesSearch({ from: 1.5, to: 2 ** 60 })).toEqual({ from: undefined, to: undefined });
    });

    it('should take the path of a resource as it is written', () => {
        expect(resourceSearch({ to: 3, path: 'resources/[qb]/qb-core' })).toEqual({
            from: undefined,
            to: 3,
            path: 'resources/[qb]/qb-core',
        });
        expect(resourceSearch({ path: 42 }).path).toBe('');
    });
});
