import { describe, expect, it } from 'vitest';

import { en } from '../i18n/en';
import { markerWords } from './MarkerOverlay';
import { bucketOf, clock, clusters, sides } from './markers';

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

const dot = (x: number, crash = false) => ({ x, crash });

const marker = (minutes: number, kind: 'start' | 'restart' | 'crash', back: number | null = null) => ({
    at: 1_761_642_000 + minutes * 60,
    kind,
    backOnlineAt: back === null ? null : 1_761_642_000 + back * 60,
});

describe('clusters', () => {
    it('should merge icons closer than the gap, chained, under the crash or else the newest', () => {
        expect(clusters([dot(100), dot(110, true), dot(124), dot(300), dot(310)], 16)).toEqual([
            { members: [0, 1, 2], shown: 1 },
            { members: [3, 4], shown: 4 },
        ]);
    });

    it('should keep icons the gap apart on their own, in order of their place', () => {
        expect(clusters([dot(200), dot(100), dot(116)], 16)).toEqual([
            { members: [1], shown: 1 },
            { members: [2], shown: 2 },
            { members: [0], shown: 0 },
        ]);
        expect(clusters([], 16)).toEqual([]);
    });
});

describe('markerWords', () => {
    it('should name the markers newest first, a crash with its return', () => {
        const words = markerWords(en, [marker(0, 'restart'), marker(90, 'crash', 92)]);

        expect(words).toBe(
            `Crash ${clock(1_761_642_000 + 5400)}, back online ${clock(1_761_642_000 + 5520)} · Restart ${clock(1_761_642_000)}`,
        );
        expect(markerWords(en, [])).toBeNull();
    });

    it('should name five markers at most and count the others', () => {
        const words = markerWords(
            en,
            Array.from({ length: 8 }, (_, index) => marker(index * 10, 'start')),
        );

        expect(words?.split(' · ')).toHaveLength(5);
        expect(words?.endsWith(' and 3 more')).toBe(true);
        expect(words?.startsWith(`Start ${clock(1_761_642_000 + 4200)}`)).toBe(true);
    });
});
