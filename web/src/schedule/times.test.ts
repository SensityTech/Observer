import { describe, expect, it } from 'vitest';

import { fillTimes, minuteOf, sortTimes } from './times';

describe('fillTimes', () => {
    it('should fill the day with times that many hours apart, ascending', () => {
        expect(fillTimes(6, '04:00')).toEqual(['04:00', '10:00', '16:00', '22:00']);
        expect(fillTimes(24, '05:30')).toEqual(['05:30']);
    });

    it('should wrap past midnight', () => {
        expect(fillTimes(8, '22:30')).toEqual(['06:30', '14:30', '22:30']);
        expect(fillTimes(12, '23:59')).toEqual(['11:59', '23:59']);
    });

    it('should fill nothing for a step that does not divide the day or a time it cannot read', () => {
        expect(fillTimes(5, '04:00')).toEqual([]);
        expect(fillTimes(6, '4:00')).toEqual([]);
    });
});

describe('times', () => {
    it('should read HH:MM on a 24-hour clock only', () => {
        expect(minuteOf('23:59')).toBe(23 * 60 + 59);
        expect(minuteOf('24:00')).toBeNull();
        expect(minuteOf('04:00 ')).toBeNull();
    });

    it('should sort as Observer stores them', () => {
        expect(sortTimes(['16:00', '04:00', '10:30'])).toEqual(['04:00', '10:30', '16:00']);
    });
});
