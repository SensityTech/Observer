import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Ellipsis, KeyRound, LogOut, ShieldCheck, Trash2, UserPlus, Users } from 'lucide-react';
import { useState } from 'react';

import type { User } from '../api/generated/User';
import { usersQuery } from '../api/users';
import { useMe } from '../auth/me';
import { canEdit, canManage, countPermissions, parsePermissions, permissionKeys } from '../auth/permissions';
import { Button, buttonVariants } from '../components/Button';
import { Chip } from '../components/Chip';
import { Menu, type MenuItem } from '../components/Menu';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { anchorOf } from './catalog';
import { CategoryHead } from './Row';
import {
    CreateUserDialog,
    DeleteUserDialog,
    EditUserDialog,
    ResetPasswordDialog,
    RevokeSessionsDialog,
    TemporaryPasswordDialog,
} from '../users/UserDialogs';

type Open =
    | { dialog: 'create' }
    | { dialog: 'edit' | 'reset' | 'revoke' | 'delete'; user: User }
    | { dialog: 'password'; title: string; password: string };

/** The accounts and what each may do, for whoever manages users: what was the Users page. */
export function UsersSettings() {
    const { t } = useI18n();
    const queryClient = useQueryClient();
    const users = useQuery(usersQuery);
    const [open, setOpen] = useState<Open | null>(null);
    const [done, setDone] = useState<string | null>(null);

    const close = () => setOpen(null);
    const changed = (message: string | null) => {
        setOpen(null);
        setDone(message);
        void queryClient.invalidateQueries({ queryKey: usersQuery.queryKey });
    };

    return (
        <>
            <CategoryHead
                category="users"
                aside={
                    <Button
                        tone="ink"
                        icon={UserPlus}
                        className="max-md:w-full"
                        onClick={() => {
                            setDone(null);
                            setOpen({ dialog: 'create' });
                        }}
                    >
                        {t.users.add}
                    </Button>
                }
            />

            {done && <Notice tone="success">{done}</Notice>}

            <section id={anchorOf('accounts')} className="card flex flex-col" aria-labelledby="users-title">
                <div className="card-head">
                    <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                        <Users size={14} strokeWidth={1.5} aria-hidden="true" />
                    </span>
                    <h3 id="users-title" className="text-card text-ink">
                        {t.users.caption}
                    </h3>
                </div>
                {users.isPending && (
                    <div className="flex flex-col gap-3 p-5" aria-busy="true">
                        <span className="sr-only">{t.common.loading}</span>
                        <span className="skeleton w-2/3" />
                        <span className="skeleton w-1/2" />
                    </div>
                )}
                {users.isError && (
                    <div className="p-5 max-md:p-4">
                        <Notice tone="danger">
                            {t.users.loadFailed} {errorMessage(t, users.error)}
                        </Notice>
                    </div>
                )}
                {users.data && (
                    <div className={table.scroll}>
                        <table className={table.table}>
                            <caption className="sr-only">{t.users.caption}</caption>
                            <thead>
                                <tr>
                                    <th scope="col" className={table.th}>
                                        {t.users.user}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {t.users.permissions}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {t.users.lastLogin}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {t.users.created}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        <span className="sr-only">{t.users.actions}</span>
                                    </th>
                                </tr>
                            </thead>
                            <tbody>
                                {users.data.map((user) => (
                                    <UserRow
                                        key={user.id}
                                        user={user}
                                        onOpen={(dialog) => {
                                            setDone(null);
                                            setOpen({ dialog, user });
                                        }}
                                    />
                                ))}
                            </tbody>
                        </table>
                    </div>
                )}
            </section>

            {open?.dialog === 'create' && (
                <CreateUserDialog
                    onClose={close}
                    onCreated={(created) => {
                        void queryClient.invalidateQueries({ queryKey: usersQuery.queryKey });
                        setOpen({
                            dialog: 'password',
                            title: t.users.temporaryPassword.created(created.user.username),
                            password: created.temporaryPassword,
                        });
                    }}
                />
            )}
            {open?.dialog === 'edit' && (
                <EditUserDialog
                    user={open.user}
                    onClose={close}
                    onSaved={() => changed(t.users.saved(open.user.username))}
                />
            )}
            {open?.dialog === 'reset' && (
                <ResetPasswordDialog
                    user={open.user}
                    onClose={close}
                    onReset={(password) => {
                        void queryClient.invalidateQueries({ queryKey: usersQuery.queryKey });
                        setOpen({
                            dialog: 'password',
                            title: t.users.temporaryPassword.reset(open.user.username),
                            password,
                        });
                    }}
                />
            )}
            {open?.dialog === 'revoke' && (
                <RevokeSessionsDialog
                    user={open.user}
                    onClose={close}
                    onRevoked={() => changed(t.users.revoked(open.user.username))}
                />
            )}
            {open?.dialog === 'delete' && (
                <DeleteUserDialog
                    user={open.user}
                    onClose={close}
                    onDeleted={() => changed(t.users.deleted(open.user.username))}
                />
            )}
            {open?.dialog === 'password' && (
                <TemporaryPasswordDialog title={open.title} password={open.password} onClose={close} />
            )}
        </>
    );
}

function UserRow({
    user,
    onOpen,
}: {
    user: User;
    onOpen: (dialog: 'edit' | 'reset' | 'revoke' | 'delete') => void;
}) {
    const { t, format } = useI18n();
    const me = useMe();
    const mask = parsePermissions(user.permissions);
    const names = permissionKeys(mask).map((key) => t.permissions[key].name);

    const actions: MenuItem[] = [];
    if (canEdit(me, user)) {
        actions.push({ label: t.users.edit, icon: ShieldCheck, onSelect: () => onOpen('edit') });
    }
    if (canManage(me, user)) {
        actions.push(
            { label: t.users.reset, icon: KeyRound, onSelect: () => onOpen('reset') },
            { label: t.users.revoke, icon: LogOut, onSelect: () => onOpen('revoke') },
            { label: t.users.delete, icon: Trash2, onSelect: () => onOpen('delete'), danger: true },
        );
    }

    return (
        <tr className={table.row}>
            <td className={table.td}>
                <span className="font-medium whitespace-nowrap text-ink">{user.username}</span>
                {(user.isOwner || user.id === me.id || user.mustChangePassword) && (
                    <div className={table.sub}>
                        {user.isOwner && <Chip>{t.users.owner}</Chip>}
                        {user.id === me.id && <Chip>{t.users.you}</Chip>}
                        {user.mustChangePassword && (
                            <Chip tone="warning" icon={KeyRound}>
                                {t.users.temporary}
                            </Chip>
                        )}
                    </div>
                )}
            </td>
            <td className={cn(table.td, 'min-w-56')}>
                {user.isOwner ? t.users.allPermissions : t.users.permissionCount(countPermissions(mask))}
                {!user.isOwner && names.length > 0 && (
                    <div className={cn(table.sub, 'block max-w-md')}>{names.join(', ')}</div>
                )}
            </td>
            <td className={cn(table.td, table.mono)}>
                {user.lastLoginAt === null ? (
                    <span className="font-sans text-ink-3">{t.users.never}</span>
                ) : (
                    format.timestamp(user.lastLoginAt, { seconds: false })
                )}
            </td>
            <td className={cn(table.td, table.mono)}>
                {format.timestamp(user.createdAt, { seconds: false })}
            </td>
            <td className={cn(table.td, 'w-[1%] text-right whitespace-nowrap')}>
                {actions.length > 0 && (
                    <Menu
                        placement="below-end"
                        items={actions}
                        label={t.users.actionsFor(user.username)}
                        buttonClassName={buttonVariants({ tone: 'ghost', size: 'icon-sm' })}
                    >
                        <Ellipsis size={16} strokeWidth={1.5} aria-hidden="true" />
                    </Menu>
                )}
            </td>
        </tr>
    );
}
