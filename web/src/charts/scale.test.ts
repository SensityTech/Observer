import { describe, expect, it } from 'vitest';

import { ceiling, scaleTop } from './scale';

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

describe('scaleTop', () => {
    it('should leave room for the most slots of the range, not only the latest', () => {
        const rows = [
            { time: 0, players: 37, slots: 64 },
            { time: 5, players: 37, slots: 512 },
            { time: 10, players: 37, slots: 64 },
        ];

        expect(scaleTop(rows)).toBe(ceiling(37, 512));
        expect(scaleTop(rows.slice(-1))).toBe(100);
    });

    it('should leave room for the peak of the players', () => {
        expect(
            scaleTop([
                { time: 0, players: 290, slots: 256 },
                { time: 5, players: null, slots: null },
            ]),
        ).toBe(500);
    });
});
