import { createContext, use } from 'react';

import type { Me } from '../api/generated/Me';
import { Permission } from '../api/generated/Permission';
import { holds, parsePermissions } from './permissions';

/** The signed-in user, provided by the panel's shell to every page under it. */
export const MeContext = createContext<Me | null>(null);

export const useMe = () => {
    const me = use(MeContext);
    if (me === null) {
        throw new Error('useMe must be used under the signed-in shell');
    }
    return me;
};

/** Whether the user holds a permission. The owner's set holds them all. */
export const allows = (me: Me, permission: bigint) => holds(parsePermissions(me.permissions), permission);

/** The first page the user may open, for a landing that is not theirs. */
export const landing = (me: Me) => {
    if (allows(me, Permission.DashboardView)) {
        return '/';
    }
    if (allows(me, Permission.UsersManage)) {
        return '/users';
    }
    return allows(me, Permission.AuditView) ? '/audit' : '/account';
};
