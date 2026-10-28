import { useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { ChevronsUpDown, LogOut, UserRound } from 'lucide-react';

import { signOut } from '../api/auth';
import type { Me } from '../api/generated/Me';
import { countPermissions, parsePermissions } from '../auth/permissions';
import { Menu } from '../components/Menu';
import { useI18n } from '../i18n/I18nProvider';

/** The signed-in user at the foot of the rail, with their account and the way out. */
export function UserMenu({ me, onNavigate }: { me: Me; onNavigate: () => void }) {
    const { t } = useI18n();
    const navigate = useNavigate();
    const queryClient = useQueryClient();

    const role = me.isOwner
        ? t.account.owner
        : t.account.permissions(countPermissions(parsePermissions(me.permissions)));

    return (
        <Menu
            placement="above-stretch"
            items={[
                {
                    label: t.account.myAccount,
                    icon: UserRound,
                    onSelect: () => {
                        onNavigate();
                        void navigate({ to: '/account' });
                    },
                },
                {
                    label: t.account.signOut,
                    icon: LogOut,
                    onSelect: () => {
                        onNavigate();
                        void signOut(queryClient).finally(() => navigate({ to: '/login' }));
                    },
                },
            ]}
            label={t.account.menu(me.username)}
            buttonClassName="user"
        >
            <span className="avatar" aria-hidden="true">
                {me.username.slice(0, 2).toUpperCase()}
            </span>
            <span className="user-meta" aria-hidden="true">
                <span className="user-name">{me.username}</span>
                <span className="user-role">{role}</span>
            </span>
            <ChevronsUpDown size={16} strokeWidth={1.5} aria-hidden="true" />
        </Menu>
    );
}
