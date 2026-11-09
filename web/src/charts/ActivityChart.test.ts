import { describe, expect, it } from 'vitest';

import { atLeastAnHour } from './ActivityChart';

describe('the activity scale', () => {
    it('should span an hour at least, so that minutes read on distinct marks', () => {
        expect(atLeastAnHour({ priceRange: { minValue: 0, maxValue: 0.03 } }).priceRange).toEqual({
            minValue: 0,
            maxValue: 1,
        });
        expect(atLeastAnHour(null).priceRange).toEqual({ minValue: 0, maxValue: 1 });
    });

    it('should keep a longer day as it is', () => {
        expect(atLeastAnHour({ priceRange: { minValue: 0, maxValue: 3.5 } }).priceRange).toEqual({
            minValue: 0,
            maxValue: 3.5,
        });
    });
});
