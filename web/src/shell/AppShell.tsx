import { useQuery } from '@tanstack/react-query';
import { Link, Outlet } from '@tanstack/react-router';
import {
    Ban,
    CalendarClock,
    ChartLine,
    Contact,
    DatabaseBackup,
    GitCompare,
    LayoutDashboard,
    Map as MapIcon,
    Menu as MenuIcon,
    Package,
    ScrollText,
    Server,
    Settings,
    SquareTerminal,
    Stethoscope,
    TriangleAlert,
    Unplug,
    Wrench,
    X,
    type LucideIcon,
} from 'lucide-react';
import { Suspense, useCallback, useEffect, useState } from 'react';

import { meQuery } from '../api/auth';
import { Permission } from '../api/generated/Permission';
import type { Me } from '../api/generated/Me';
import { allows, MeContext } from '../auth/me';
import { buttonVariants } from '../components/Button';
import { Chip } from '../components/Chip';
import { PlainHttpNotice } from '../components/PlainHttpNotice';
import { AlertsProvider, useAlerts } from '../diagnostics/alerts';
import { useI18n } from '../i18n/I18nProvider';
import { useRealtime } from '../live/context';
import { cn } from '../lib/cn';
import { PHONE, useMedia } from '../lib/useMedia';
import { ArtifactsProvider } from '../artifacts/context';
import { ScheduleProvider } from '../schedule/context';
import { ServerStatusProvider, useServerStatus } from '../server/context';
import { Logo } from './Logo';
import { OnlineButton, OnlineColumn } from './OnlineColumn';
import { ServerControls } from './ServerControls';
import { StatusBar } from './StatusBar';
import { UserMenu } from './UserMenu';

type NavEntry = {
    to:
        | '/'
        | '/statistics'
        | '/console'
        | '/schedule'
        | '/artifacts'
        | '/diagnostics'
        | '/players'
        | '/map'
        | '/bans'
        | '/changes'
        | '/drops'
        | '/settings'
        | '/tools'
        | '/backups'
        | '/audit';
    label: string;
    icon: LucideIcon;
    /**
     * A permission, any one of a list, the owner alone for what no permission grants, or every
     * account.
     */
    access: bigint | bigint[] | 'owner' | 'everyone';
    /** Shows how many alerts are live. */
    alerts?: boolean;
    /**
     * Stays current on the pages under it, as a player's profile is under the players and a resource
     * under the changes, and whatever its address holds, as the drops' filters.
     */
    nested?: boolean;
};

export function AppShell() {
    const { t } = useI18n();
    const { data: me } = useQuery(meQuery);
    const [menuOpen, setMenuOpen] = useState(false);
    const [playersOpen, setPlayersOpen] = useState(false);
    const closeMenu = () => setMenuOpen(false);
    const openPlayers = useCallback(() => setPlayersOpen(true), []);
    const closePlayers = useCallback(() => setPlayersOpen(false), []);
    const phone = useMedia(PHONE);
    const live = useRealtime();

    // The socket lives as long as the signed-in shell: signing out unmounts it, and closes it.
    useEffect(() => {
        live.start();
        return () => live.stop();
    }, [live]);

    useEffect(() => {
        if (!menuOpen && !playersOpen) {
            return;
        }
        const root = document.documentElement;
        root.classList.add('drawer-open');
        // Also when the shell goes away with a drawer open, signing out from it: the sign-in page must scroll.
        return () => root.classList.remove('drawer-open');
    }, [menuOpen, playersOpen]);

    useEffect(() => {
        if (!menuOpen) {
            return;
        }
        const closeOnEscape = (event: KeyboardEvent) => {
            if (event.key === 'Escape') {
                setMenuOpen(false);
            }
        };
        window.addEventListener('keydown', closeOnEscape);
        return () => window.removeEventListener('keydown', closeOnEscape);
    }, [menuOpen]);

    // Signed out under the shell: the route guard is already on its way to the sign-in page.
    if (!me) {
        return null;
    }
    const players = allows(me, Permission.PlayersView);

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
        { to: '/players', label: t.nav.players, icon: Contact, access: Permission.PlayersView, nested: true },
        { to: '/map', label: t.nav.map, icon: MapIcon, access: Permission.PlayersMap },
        { to: '/bans', label: t.nav.bans, icon: Ban, access: Permission.BansView },
        { to: '/statistics', label: t.nav.statistics, icon: ChartLine, access: Permission.DashboardView },
        {
            to: '/diagnostics',
            label: t.nav.diagnostics,
            icon: Stethoscope,
            access: Permission.DiagnosticsView,
            alerts: true,
        },
        {
            to: '/changes',
            label: t.nav.changes,
            icon: GitCompare,
            access: Permission.ChangesView,
            nested: true,
        },
        { to: '/drops', label: t.nav.drops, icon: Unplug, access: Permission.DropsView, nested: true },
    ];
    const maintenance: NavEntry[] = [
        { to: '/artifacts', label: t.nav.artifacts, icon: Package, access: Permission.ArtifactsView },
        { to: '/backups', label: t.nav.backups, icon: DatabaseBackup, access: Permission.BackupsView },
        { to: '/tools', label: t.nav.tools, icon: Wrench, access: 'owner' },
        { to: '/audit', label: t.nav.audit, icon: ScrollText, access: Permission.AuditView },
    ];
    // At the foot, beside the account: every account opens its own settings, and the categories
    // its permissions open, the users and the file access among them.
    const settings: NavEntry[] = [
        { to: '/settings', label: t.nav.settings, icon: Settings, access: 'everyone', nested: true },
    ];

    return (
        <MeContext value={me}>
            <ServerStatusProvider>
                <ScheduleProvider>
                    <ArtifactsProvider>
                        <AlertsProvider>
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
                                        <ServerTile />
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
                                                entries={maintenance}
                                                group={t.nav.maintenance}
                                                onNavigate={closeMenu}
                                            />
                                        </nav>
                                        <div className="rail-foot">
                                            <NavEntries me={me} entries={settings} onNavigate={closeMenu} />
                                            <UserMenu me={me} onNavigate={closeMenu} />
                                        </div>
                                    </div>
                                </aside>
                                {menuOpen && <div className="scrim" aria-hidden="true" onClick={closeMenu} />}

                                <div className="main" inert={menuOpen || playersOpen}>
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
                                        {players && phone && (
                                            <OnlineButton open={playersOpen} onOpen={openPlayers} />
                                        )}
                                    </div>
                                    <header className="topbar" aria-label={t.status.label}>
                                        <div className="status-card">
                                            <StatusBar />
                                            <ServerControls />
                                        </div>
                                    </header>
                                    <main className="content" id="content" tabIndex={-1}>
                                        <PlainHttpNotice />
                                        <Suspense fallback={<PageLoading />}>
                                            <Outlet />
                                        </Suspense>
                                    </main>
                                </div>

                                {players && (
                                    <OnlineColumn
                                        open={playersOpen}
                                        onOpen={openPlayers}
                                        onClose={closePlayers}
                                    />
                                )}
                            </div>
                        </AlertsProvider>
                    </ArtifactsProvider>
                </ScheduleProvider>
            </ServerStatusProvider>
        </MeContext>
    );
}

/**
 * The server Observer runs, by the name and port it last ran with. A tile, not a switcher: Observer
 * runs one server.
 */
function ServerTile() {
    const { t } = useI18n();
    const { status } = useServerStatus();
    const name = status?.name ?? null;
    const port = status?.port ?? null;
    return (
        <div className="server-tile">
            <span className="server-tile-mark" aria-hidden="true">
                {name === null ? (
                    <Server size={14} strokeWidth={1.75} />
                ) : (
                    [...name.replace(/[^\p{L}\p{N}]/gu, '')].slice(0, 2).join('').toUpperCase()
                )}
            </span>
            <span className="server-tile-meta">
                <span className="server-tile-name">{name ?? t.shell.server}</span>
                <span className="server-tile-port">
                    {port === null ? t.shell.serverUnknown : t.shell.port(String(port))}
                </span>
            </span>
        </div>
    );
}

/** A page whose code is still loading: its heading and a card, in their places. */
function PageLoading() {
    const { t } = useI18n();
    return (
        <div className="flex flex-col gap-3" aria-busy="true" aria-label={t.common.loading}>
            <div className="skeleton h-7 w-48" />
            <div className="skeleton h-64 w-full rounded-card" />
        </div>
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
        access === 'everyone'
            ? true
            : access === 'owner'
              ? me.isOwner
              : (Array.isArray(access) ? access : [access]).some((permission) => allows(me, permission)),
    );
    if (allowed.length === 0) {
        return null;
    }
    return (
        <>
            {group && <p className="nav-group">{group}</p>}
            {allowed.map(({ to, label, icon: Icon, alerts, nested }) => (
                <Link
                    key={to}
                    to={to}
                    className="nav-item"
                    activeOptions={{ exact: nested !== true, includeSearch: false }}
                    onClick={onNavigate}
                >
                    <Icon size={16} strokeWidth={1.5} aria-hidden="true" />
                    {label}
                    {alerts === true && <AlertCount />}
                </Link>
            ))}
        </>
    );
}

/** The live alerts' count on the Diagnostics entry, while there is any. */
function AlertCount() {
    const { t, format } = useI18n();
    const count = useAlerts().length;
    if (count === 0) {
        return null;
    }
    return (
        <Chip tone="warning" icon={TriangleAlert} className="ml-auto">
            <span aria-hidden="true">{format.number(count)}</span>
            <span className="sr-only">{t.nav.alertCount(count)}</span>
        </Chip>
    );
}
