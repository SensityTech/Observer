import { describe, expect, it } from 'vitest';

import type { Me } from '../api/generated/Me';
import { allows, landing } from './me';
import { Permission } from '../api/generated/Permission';

const me = (permissions: string, isOwner = false): Me => ({
    id: 2,
    username: 'kenji',
    isOwner,
    permissions,
    mustChangePassword: false,
});

describe('allows', () => {
    it('should read the hexadecimal set against a generated constant', () => {
        expect(allows(me('101'), Permission.AuditView)).toBe(true);
        expect(allows(me('101'), Permission.UsersManage)).toBe(false);
        expect(allows(me('fff', true), Permission.LogsDownload)).toBe(true);
    });
});

describe('landing', () => {
    it('should open the overview for whoever may see the dashboard', () => {
        expect(landing(me('fff', true))).toBe('/');
        expect(landing(me('1'))).toBe('/');
    });

    it('should fall back to the first page the user may open, then to their account', () => {
        expect(landing(me('180'))).toBe('/users');
        expect(landing(me('100'))).toBe('/audit');
        expect(landing(me('2'))).toBe('/account');
        expect(landing(me('0'))).toBe('/account');
    });
});
