import { useQuery } from '@tanstack/react-query';
import { Link, Outlet } from '@tanstack/react-router';
import { Menu as MenuIcon, Search, Server, TriangleAlert, X } from 'lucide-react';
import { Suspense, useCallback, useEffect, useState } from 'react';

import { meQuery } from '../api/auth';
import { Permission } from '../api/generated/Permission';
import type { Me } from '../api/generated/Me';
import { allows } from '../auth/me';
import { buttonVariants } from '../components/Button';
import { Chip } from '../components/Chip';
import { PlainHttpNotice } from '../components/PlainHttpNotice';
import { useAlerts } from '../diagnostics/alerts';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { PHONE, useMedia } from '../lib/useMedia';
import { useServerStatus } from '../server/context';
import { Logo } from './Logo';
import { OnlineButton, OnlineColumn } from './OnlineColumn';
import { ServerControls } from './ServerControls';
import { SignedIn } from './SignedIn';
import { StatusBar } from './StatusBar';
import { UpdateSignal } from '../updates/UpdateSignal';
import { CommandPalette } from './CommandPalette';
import { navSections, opensEntry, type NavEntry } from './nav';
import { UserMenu } from './UserMenu';

export function AppShell() {
    const { t } = useI18n();
    const { data: me } = useQuery(meQuery);
    const [menuOpen, setMenuOpen] = useState(false);
    const [playersOpen, setPlayersOpen] = useState(false);
    const [paletteOpen, setPaletteOpen] = useState(false);
    const openPalette = useCallback(() => setPaletteOpen(true), []);
    const closePalette = useCallback(() => setPaletteOpen(false), []);
    const closeMenu = () => setMenuOpen(false);
    const openPlayers = useCallback(() => setPlayersOpen(true), []);
    const closePlayers = useCallback(() => setPlayersOpen(false), []);
    const phone = useMedia(PHONE);

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

    const sections = navSections(t);

    return (
        <SignedIn>
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
                                className="ml-auto inline-flex h-8 items-center gap-2 rounded-control border border-line bg-surface-2 px-2 text-ink-3 hover:text-ink max-md:h-11"
                                aria-label={t.palette.label}
                                aria-keyshortcuts="Control+K"
                                title={t.palette.label}
                                onClick={() => {
                                    closeMenu();
                                    openPalette();
                                }}
                            >
                                <Search size={14} strokeWidth={1.5} aria-hidden="true" />
                                <kbd className="font-mono text-micro">Ctrl K</kbd>
                            </button>
                            <button
                                type="button"
                                className={cn(buttonVariants({ tone: 'ghost', size: 'icon' }), 'xl:hidden')}
                                aria-label={t.nav.close}
                                onClick={closeMenu}
                            >
                                <X size={16} strokeWidth={1.5} aria-hidden="true" />
                            </button>
                        </div>
                        <ServerTile />
                        <nav className="nav" aria-label={t.nav.sections}>
                            <NavEntries me={me} entries={sections.main} onNavigate={closeMenu} />
                            <NavEntries
                                me={me}
                                entries={sections.people}
                                group={t.nav.groupPlayers}
                                onNavigate={closeMenu}
                            />
                            <NavEntries
                                me={me}
                                entries={sections.server}
                                group={t.nav.groupServer}
                                onNavigate={closeMenu}
                            />
                            <NavEntries
                                me={me}
                                entries={sections.insights}
                                group={t.nav.groupInsights}
                                onNavigate={closeMenu}
                            />
                            <NavEntries
                                me={me}
                                entries={sections.maintenance}
                                group={t.nav.maintenance}
                                onNavigate={closeMenu}
                            />
                        </nav>
                        <div className="rail-foot">
                            {me.isOwner && <UpdateSignal onNavigate={closeMenu} />}
                            <NavEntries me={me} entries={sections.settings} onNavigate={closeMenu} />
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
                        {players && phone && <OnlineButton open={playersOpen} onOpen={openPlayers} />}
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

                {players && <OnlineColumn open={playersOpen} onOpen={openPlayers} onClose={closePlayers} />}
                <CommandPalette me={me} open={paletteOpen} onOpen={openPalette} onClose={closePalette} />
            </div>
        </SignedIn>
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
    const allowed = entries.filter((entry) => opensEntry(me, entry));
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
