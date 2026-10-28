import { describe, expect, it } from 'vitest';

import { bucketOf, sides } from './markers';

const at = (x: number, width: number, weight = 0) => ({ x, width, weight });

describe('sides', () => {
    it('should put a label left of its instant when it fits there', () => {
        expect(sides([at(300, 120)], 1000)).toEqual(['left']);
    });

    it('should move a label right of its instant at the plot edge or against another label', () => {
        expect(sides([at(60, 100), at(400, 120), at(450, 200), at(900, 100)], 1000)).toEqual([
            'right',
            'left',
            'right',
            'left',
        ]);
    });

    it('should give the room to a crash first and hide what fits nowhere', () => {
        // A start, a crash four minutes later and the start after it, on an hour 1,000 px wide.
        expect(sides([at(880, 86), at(942, 200, 1), at(972, 86)], 1000)).toEqual([
            'hidden',
            'left',
            'hidden',
        ]);
        expect(sides([at(300, 86), at(302, 224)], 343)).toEqual(['left', 'hidden']);
    });
});

describe('bucketOf', () => {
    it('should place a marker on the start of the bucket holding it', () => {
        expect(bucketOf({ at: 1_000_077, kind: 'crash', backOnlineAt: null }, 1_000_000, 30)).toBe(1_000_060);
    });
});
