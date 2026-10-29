import { describe, expect, it } from 'vitest';

import { isRunGrowth, isThreadShares } from './metrics';

const shares = {
    from: 1_000_000,
    to: 1_003_600,
    measured: 720,
    process: 90.5,
    threads: [
        { name: 'UV loop: svMain', group: 'main', cpu: 58 },
        { name: '', group: 'other', cpu: 0.2 },
    ],
};

const growth = {
    startedAt: 1_000_000,
    from: 1_001_800,
    to: 1_009_000,
    private: { perHour: 188_743_680, fit: 0.97, start: 1e9, points: 120, leak: true },
    handles: null,
};

describe('metrics answers', () => {
    it('should take thread shares field by field', () => {
        expect(isThreadShares(shares)).toBe(true);
        expect(isThreadShares({ ...shares, process: null, threads: [] })).toBe(true);
        expect(isThreadShares({ ...shares, measured: '720' })).toBe(false);
        expect(isThreadShares({ ...shares, threads: [{ name: 'x', group: 'gpu', cpu: 1 }] })).toBe(false);
        expect(isThreadShares({ ...shares, threads: [{ name: 'x', group: 'main', cpu: null }] })).toBe(false);
        expect(isThreadShares(null)).toBe(false);
    });

    it('should take a run growth, or null without a run', () => {
        expect(isRunGrowth(growth)).toBe(true);
        expect(isRunGrowth(null)).toBe(true);
        expect(isRunGrowth({ ...growth, private: { ...growth.private, leak: 'yes' } })).toBe(false);
        expect(isRunGrowth({ ...growth, handles: undefined })).toBe(false);
        expect(isRunGrowth([])).toBe(false);
    });
});
