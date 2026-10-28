import { describe, expect, it } from 'vitest';

import { secondsSince } from './useNow';

describe('secondsSince', () => {
    const start = '2026-10-28T14:00:00Z';
    const at = (seconds: number) => Date.parse(start) + seconds * 1000;

    it('should count whole seconds, then whole minutes', () => {
        expect(secondsSince(start, at(59.9))).toBe(59);
        expect(secondsSince(start, at(61))).toBe(60);
        expect(secondsSince(start, at(119))).toBe(60);
        expect(secondsSince(start, at(2 * 3600 + 14 * 60 + 30))).toBe(2 * 3600 + 14 * 60);
    });

    it('should never read a start in the future', () => {
        expect(secondsSince(start, at(-5))).toBe(0);
    });
});
