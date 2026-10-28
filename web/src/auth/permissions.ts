import { Permission } from '../api/generated/Permission';
import type { PermissionInfo } from '../api/generated/PermissionInfo';
import type { Permissions } from '../api/generated/Permissions';

// Permission masks are BigInts end to end: the catalog is append-only and may outgrow 53 bits.

export type PermissionName = keyof typeof Permission;
/** A permission's key in the catalog and in the message catalogs, such as `consoleWrite`. */
export type PermissionKey = Uncapitalize<PermissionName>;

const NAMES = Object.keys(Permission) as PermissionName[];

export const keyOf = (name: PermissionName) =>
    (name.charAt(0).toLowerCase() + name.slice(1)) as PermissionKey;

/** Reads the API's hexadecimal form. Anything that is not hexadecimal grants nothing. */
export const parsePermissions = (hex: Permissions): bigint =>
    /^[0-9a-f]+$/i.test(hex) ? BigInt(`0x${hex}`) : 0n;

export const formatPermissions = (mask: bigint): Permissions => mask.toString(16);

export const bitMask = (bit: number) => 1n << BigInt(bit);

export const holds = (mask: bigint, permission: bigint) => (mask & permission) === permission;

/** The keys of the generated constants a mask holds, in bit order. */
export const permissionKeys = (mask: bigint): PermissionKey[] =>
    NAMES.filter((name) => holds(mask, Permission[name])).map(keyOf);

export const countPermissions = (mask: bigint) => {
    let count = 0;
    for (let rest = mask; rest !== 0n; rest &= rest - 1n) {
        count += 1;
    }
    return count;
};

/** Adds everything the mask's permissions imply, directly or not, as the API does on write. */
export const closeImplications = (mask: bigint, catalog: readonly PermissionInfo[]) => {
    let closed = mask;
    for (;;) {
        const next = catalog
            .filter((info) => holds(closed, bitMask(info.bit)))
            .reduce((set, info) => set | parsePermissions(info.implies), closed);
        if (next === closed) {
            return closed;
        }
        closed = next;
    }
};

/**
 * Grants or withdraws one permission. Granting brings what it implies; withdrawing also withdraws
 * whatever implies it, so the set stays closed.
 */
export const setPermission = (
    mask: bigint,
    info: PermissionInfo,
    granted: boolean,
    catalog: readonly PermissionInfo[],
) => {
    const bit = bitMask(info.bit);
    if (granted) {
        return closeImplications(mask | bit, catalog);
    }
    return catalog
        .filter((other) => holds(closeImplications(bitMask(other.bit), catalog), bit))
        .reduce((set, other) => set & ~bitMask(other.bit), mask);
};

/** The catalog entries already granted by another permission of the mask. */
export const impliedBy = (mask: bigint, info: PermissionInfo, catalog: readonly PermissionInfo[]) =>
    catalog.filter(
        (other) =>
            other.bit !== info.bit &&
            holds(mask, bitMask(other.bit)) &&
            holds(closeImplications(bitMask(other.bit), catalog), bitMask(info.bit)),
    );

type Account = { id: number; isOwner: boolean; permissions: Permissions };

/** What an editor may grant or withdraw: the owner anything, anyone else only what they hold. */
export const grantable = (editor: Account, catalog: readonly PermissionInfo[]) =>
    editor.isOwner
        ? catalog.reduce((set, info) => set | bitMask(info.bit), 0n)
        : parsePermissions(editor.permissions);

/** Whether the editor may change the target's permissions. The API decides; this only hides dead ends. */
export const canEdit = (editor: Account, target: Account) => editor.id !== target.id && !target.isOwner;

/** Delete, reset and revoke: the owner, or an editor holding every permission the target holds. */
export const canManage = (editor: Account, target: Account) =>
    canEdit(editor, target) &&
    (editor.isOwner || holds(parsePermissions(editor.permissions), parsePermissions(target.permissions)));
