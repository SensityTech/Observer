import { describe, expect, it } from 'vitest';

import type { BanEdit } from '../api/generated/BanEdit';
import { minutesOf } from './words';

const CREATED = '2026-11-09T20:00:00Z';
const reworded: BanEdit = {
    at: '2026-11-10T09:00:00Z',
    by: 'maxime',
    reason: { before: 'Aimbot', after: 'Aimbot, two clips' },
    ends: null,
};
const extended: BanEdit = {
    at: '2026-11-10T08:30:00Z',
    by: 'sarah',
    reason: null,
    ends: { before: '2026-11-10T20:00:00Z', after: '2026-11-17T08:30:00Z' },
};

describe('minutesOf', () => {
    it('should count a ban no one edited from when it was made', () => {
        expect(minutesOf({ createdAt: CREATED, endsAt: '2026-11-10T20:00:00Z', edits: [] })).toBe(24 * 60);
        expect(minutesOf({ createdAt: CREATED, endsAt: null, edits: [] })).toBeNull();
    });

    it('should count from the latest edit that gave the ban its end', () => {
        const ban = { createdAt: CREATED, endsAt: '2026-11-17T08:30:00Z', edits: [reworded, extended] };

        expect(minutesOf(ban)).toBe(7 * 24 * 60);
        expect(minutesOf({ ...ban, edits: [reworded] })).toBe(7 * 24 * 60 + 12 * 60 + 30);
    });
});
