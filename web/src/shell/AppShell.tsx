import { useQuery } from '@tanstack/react-query';
import { Link, Outlet } from '@tanstack/react-router';
import {
    CalendarClock,
    LayoutDashboard,
    Menu as MenuIcon,
    ScrollText,
    Server,
    SquareTerminal,
    Users,
    X,
    type LucideIcon,
} from 'lucide-react';
import { useEffect, useState } from 'react';

import { meQuery } from '../api/auth';
import { Permission } from '../api/generated/Permission';
import type { Me } from '../api/generated/Me';
import { allows, MeContext } from '../auth/me';
import { buttonVariants } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';
import { useRealtime } from '../live/context';
import { cn } from '../lib/cn';
import { ScheduleProvider } from '../schedule/context';
import { ServerStatusProvider } from '../server/context';
import { Logo } from './Logo';
import { ServerControls } from './ServerControls';
import { StatusBar } from './StatusBar';
import { ThemeSwitch } from './ThemeSwitch';
import { UserMenu } from './UserMenu';

type NavEntry = {
    to: '/' | '/console' | '/schedule' | '/server' | '/users' | '/audit';
    label: string;
    icon: LucideIcon;
    /** A permission, any one of a list, or the owner alone for what no permission grants. */
    access: bigint | bigint[] | 'owner';
};

export function AppShell() {
    const { t } = useI18n();
    const { data: me } = useQuery(meQuery);
    const [menuOpen, setMenuOpen] = useState(false);
    const closeMenu = () => setMenuOpen(false);
    const live = useRealtime();

    // The socket lives as long as the signed-in shell: signing out unmounts it, and closes it.
    useEffect(() => {
        live.start();
        return () => live.stop();
    }, [live]);

    useEffect(() => {
        if (!menuOpen) {
            return;
        }
        const root = document.documentElement;
        root.classList.add('drawer-open');
        const closeOnEscape = (event: KeyboardEvent) => {
            if (event.key === 'Escape') {
                setMenuOpen(false);
            }
        };
        window.addEventListener('keydown', closeOnEscape);
        // Also when the shell goes away with the drawer open, signing out from it: the sign-in page must scroll.
        return () => {
            window.removeEventListener('keydown', closeOnEscape);
            root.classList.remove('drawer-open');
        };
    }, [menuOpen]);

    // Signed out under the shell: the route guard is already on its way to the sign-in page.
    if (!me) {
        return null;
    }

    const main: NavEntry[] = [
        { to: '/', label: t.nav.overview, icon: LayoutDashboard, access: Permission.DashboardView },
        { to: '/console', label: t.nav.console, icon: SquareTerminal, access: Permission.ConsoleView },
    ];
    const operations: NavEntry[] = [
        {
            to: '/schedule',
            label: t.nav.schedule,
            icon: CalendarClock,
            access: [Permission.DashboardView, Permission.ScheduleManage, Permission.ServerControl],
        },
    ];
    const administration: NavEntry[] = [
        { to: '/server', label: t.nav.server, icon: Server, access: 'owner' },
        { to: '/users', label: t.nav.users, icon: Users, access: Permission.UsersManage },
        { to: '/audit', label: t.nav.audit, icon: ScrollText, access: Permission.AuditView },
    ];

    return (
        <MeContext value={me}>
            <ServerStatusProvider>
                <ScheduleProvider>
                    <div className="app">
                        <a className="skip" href="#content">
                            {t.app.skipToContent}
                        </a>

                        <aside
                            id="rail"
                            className={menuOpen ? 'rail-wrap is-open' : 'rail-wrap'}
                            aria-label={t.nav.label}
                        >
                            <div className="rail">
                                <div className="rail-head">
                                    <Logo />
                                    <button
                                        type="button"
                                        className={cn(
                                            buttonVariants({ tone: 'ghost', size: 'icon' }),
                                            'ml-auto xl:hidden',
                                        )}
                                        aria-label={t.nav.close}
                                        onClick={closeMenu}
                                    >
                                        <X size={16} strokeWidth={1.5} aria-hidden="true" />
                                    </button>
                                </div>
                                <nav className="nav" aria-label={t.nav.sections}>
                                    <NavEntries me={me} entries={main} onNavigate={closeMenu} />
                                    <NavEntries
                                        me={me}
                                        entries={operations}
                                        group={t.nav.operations}
                                        onNavigate={closeMenu}
                                    />
                                    <NavEntries
                                        me={me}
                                        entries={administration}
                                        group={t.nav.administration}
                                        onNavigate={closeMenu}
                                    />
                                </nav>
                                <div className="rail-foot">
                                    <ThemeSwitch />
                                    <UserMenu me={me} onNavigate={closeMenu} />
                                </div>
                            </div>
                        </aside>
                        {menuOpen && <div className="scrim" aria-hidden="true" onClick={closeMenu} />}

                        <div className="main" inert={menuOpen}>
                            <div className="mobilebar">
                                <button
                                    type="button"
                                    className={buttonVariants({ tone: 'ghost', size: 'icon' })}
                                    aria-label={t.nav.open}
                                    aria-controls="rail"
                                    aria-expanded={menuOpen}
                                    onClick={() => setMenuOpen(true)}
                                >
                                    <MenuIcon size={18} strokeWidth={1.5} aria-hidden="true" />
                                </button>
                                <Logo />
                            </div>
                            <header className="topbar" aria-label={t.status.label}>
                                <StatusBar />
                                <ServerControls />
                            </header>
                            <main className="content" id="content" tabIndex={-1}>
                                <Outlet />
                            </main>
                        </div>
                    </div>
                </ScheduleProvider>
            </ServerStatusProvider>
        </MeContext>
    );
}

/** The entries the user may open; a group without any is left out, heading included. */
function NavEntries({
    me,
    entries,
    group,
    onNavigate,
}: {
    me: Me;
    entries: NavEntry[];
    group?: string;
    onNavigate: () => void;
}) {
    const allowed = entries.filter(({ access }) =>
        access === 'owner'
            ? me.isOwner
            : (Array.isArray(access) ? access : [access]).some((permission) => allows(me, permission)),
    );
    if (allowed.length === 0) {
        return null;
    }
    return (
        <>
            {group && <p className="nav-group">{group}</p>}
            {allowed.map(({ to, label, icon: Icon }) => (
                <Link
                    key={to}
                    to={to}
                    className="nav-item"
                    activeOptions={{ exact: true }}
                    onClick={onNavigate}
                >
                    <Icon size={16} strokeWidth={1.5} aria-hidden="true" />
                    {label}
                </Link>
            ))}
        </>
    );
}
