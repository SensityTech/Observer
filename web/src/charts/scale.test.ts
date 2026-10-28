import { describe, expect, it } from 'vitest';

import { ceiling } from './scale';

describe('ceiling', () => {
    it('should round above the larger of peak and slots plus a tenth', () => {
        expect(ceiling(187, 256)).toBe(300);
        expect(ceiling(290, 256)).toBe(500);
        expect(ceiling(3, 8)).toBe(10);
        expect(ceiling(12, 48)).toBe(100);
        expect(ceiling(1500, 2048)).toBe(3000);
    });

    it('should keep a scale when nothing is known', () => {
        expect(ceiling(0, null)).toBe(10);
    });
});
