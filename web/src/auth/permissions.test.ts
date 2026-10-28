import { describe, expect, it } from 'vitest';

import { Permission } from '../api/generated/Permission';
import type { PermissionInfo } from '../api/generated/PermissionInfo';
import {
    canEdit,
    canManage,
    closeImplications,
    countPermissions,
    formatPermissions,
    grantable,
    holds,
    impliedBy,
    parsePermissions,
    permissionKeys,
    setPermission,
} from './permissions';

// The catalog as GET /api/permissions sends it, cut to the entries the rules need.
const info = (bit: number, key: string, implies = '0'): PermissionInfo => ({
    bit,
    key,
    area: 'console',
    danger: 'normal',
    implies,
});
const catalog = [
    info(0, 'dashboardView'),
    info(1, 'consoleView'),
    info(2, 'consoleWrite', '2'),
    info(7, 'usersManage'),
    info(8, 'auditView'),
    info(9, 'settingsView'),
    info(10, 'settingsManage', '200'),
];
const [dashboardView, consoleView, consoleWrite, , , settingsView, settingsManage] = catalog as [
    PermissionInfo,
    PermissionInfo,
    PermissionInfo,
    PermissionInfo,
    PermissionInfo,
    PermissionInfo,
    PermissionInfo,
];

describe('the wire form', () => {
    it('should read the hexadecimal the API sends as a BigInt mask', () => {
        expect(parsePermissions('fff')).toBe(0xfffn);
        expect(parsePermissions('182')).toBe(
            Permission.UsersManage | Permission.AuditView | Permission.ConsoleView,
        );
        expect(parsePermissions('0')).toBe(0n);
    });

    it('should grant nothing for a value that is not hexadecimal', () => {
        expect(parsePermissions('')).toBe(0n);
        expect(parsePermissions('0x10')).toBe(0n);
        expect(parsePermissions('zz')).toBe(0n);
    });

    it('should write lowercase hexadecimal without a prefix', () => {
        expect(formatPermissions(Permission.ConsoleWrite | Permission.ConsoleView)).toBe('6');
        expect(formatPermissions(Permission.SettingsManage | Permission.LogsDownload)).toBe('c00');
        expect(formatPermissions(0n)).toBe('0');
    });

    it('should keep bits past 53 intact, where a Number would round them', () => {
        const wide = (1n << 100n) | Permission.DashboardView;
        expect(parsePermissions(formatPermissions(wide))).toBe(wide);
    });
});

describe('reading a mask', () => {
    it('should hold a permission only when every bit of it is set', () => {
        const mask = Permission.ConsoleView | Permission.AuditView;
        expect(holds(mask, Permission.AuditView)).toBe(true);
        expect(holds(mask, Permission.ConsoleWrite)).toBe(false);
        expect(holds(mask, Permission.ConsoleView | Permission.ConsoleWrite)).toBe(false);
    });

    it('should list the held permissions by catalog key, in bit order', () => {
        expect(permissionKeys(Permission.AuditView | Permission.DashboardView)).toEqual([
            'dashboardView',
            'auditView',
        ]);
        expect(countPermissions(parsePermissions('fff'))).toBe(12);
        expect(countPermissions(0n)).toBe(0);
    });
});

describe('implications', () => {
    it('should add what a permission implies, as the API closes a set on write', () => {
        expect(closeImplications(Permission.ConsoleWrite, catalog)).toBe(
            Permission.ConsoleWrite | Permission.ConsoleView,
        );
        expect(closeImplications(Permission.DashboardView, catalog)).toBe(Permission.DashboardView);
    });

    it('should bring the implied permission along when one is granted', () => {
        expect(setPermission(0n, consoleWrite, true, catalog)).toBe(
            Permission.ConsoleWrite | Permission.ConsoleView,
        );
    });

    it('should withdraw what implies a permission along with it', () => {
        const mask = Permission.ConsoleWrite | Permission.ConsoleView | Permission.DashboardView;
        expect(setPermission(mask, consoleView, false, catalog)).toBe(Permission.DashboardView);
    });

    it('should keep the implied permission when only the one implying it is withdrawn', () => {
        const mask = Permission.ConsoleWrite | Permission.ConsoleView;
        expect(setPermission(mask, consoleWrite, false, catalog)).toBe(Permission.ConsoleView);
    });

    it('should name the permissions a granted one comes with', () => {
        const mask = Permission.SettingsManage | Permission.SettingsView;
        expect(impliedBy(mask, settingsView, catalog)).toEqual([settingsManage]);
        expect(impliedBy(mask, settingsManage, catalog)).toEqual([]);
        expect(impliedBy(Permission.SettingsView, settingsView, catalog)).toEqual([]);
        expect(impliedBy(mask, dashboardView, catalog)).toEqual([]);
    });
});

describe('delegation', () => {
    const owner = { id: 1, isOwner: true, permissions: 'fff' };
    const moderator = { id: 2, isOwner: false, permissions: '183' };
    const viewer = { id: 3, isOwner: false, permissions: '3' };
    const auditor = { id: 4, isOwner: false, permissions: '100' };

    it('should let the owner grant the whole catalog and anyone else only what they hold', () => {
        expect(grantable(owner, catalog)).toBe(0x787n);
        expect(grantable(moderator, catalog)).toBe(parsePermissions('183'));
    });

    it('should refuse editing oneself or the owner', () => {
        expect(canEdit(moderator, moderator)).toBe(false);
        expect(canEdit(moderator, owner)).toBe(false);
        expect(canEdit(owner, moderator)).toBe(true);
        expect(canEdit(moderator, viewer)).toBe(true);
    });

    it('should let a sub-user delete, reset or revoke only an account whose permissions they all hold', () => {
        expect(canManage(moderator, viewer)).toBe(true);
        expect(canManage(viewer, auditor)).toBe(false);
        expect(canManage(moderator, auditor)).toBe(true);
        expect(canManage(owner, auditor)).toBe(true);
        expect(canManage(moderator, owner)).toBe(false);
        expect(canManage(owner, owner)).toBe(false);
    });
});
