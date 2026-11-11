import { describe, expect, it } from 'vitest';

import { MAX_MINUTES } from '../api/bans';
import { A_DAY, draftMinutes, draftOf } from './fields';

describe('ban lengths', () => {
    it('should open a stored length as a usual one, or in its largest unit that counts it whole', () => {
        expect(draftOf(null).length).toBe('permanent');
        expect(draftOf(7 * 24 * 60).length).toBe('week');
        expect(draftOf(45 * 24 * 60)).toEqual({ length: 'other', amount: '45', unit: 'days' });
        expect(draftOf(5 * 60)).toEqual({ length: 'other', amount: '5', unit: 'hours' });
        expect(draftOf(90)).toEqual({ length: 'other', amount: '90', unit: 'minutes' });
    });

    it('should read back the minutes of each stored length', () => {
        for (const minutes of [null, 1, 60, 90, 5 * 60, 24 * 60, 45 * 24 * 60, MAX_MINUTES]) {
            expect(draftMinutes(draftOf(minutes))).toBe(minutes);
        }
    });

    it('should take no length from an amount no ban lasts, or from an end left as it is', () => {
        for (const amount of ['', '0', '-2', 'soon', String(MAX_MINUTES + 1)]) {
            expect(draftMinutes({ length: 'other', amount, unit: 'minutes' })).toBeUndefined();
        }
        expect(draftMinutes({ ...A_DAY, length: 'keep' })).toBeUndefined();
        expect(draftMinutes(A_DAY)).toBe(24 * 60);
    });
});
