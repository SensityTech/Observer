import { useQuery } from '@tanstack/react-query';
import { Link, Outlet, useRouter, useRouterState } from '@tanstack/react-router';
import { ChevronRight, Menu as MenuIcon, Search, Server, TriangleAlert, X } from 'lucide-react';
import { Suspense, useCallback, useEffect, useRef, useState } from 'react';

import { useAdmission } from '../admission/context';
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
import { MaintenanceBanner } from './Maintenance';
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
    const nav = useCurrentEntryInSight();

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
                        <nav ref={nav} className="nav" aria-label={t.nav.sections}>
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
                            <NavFold
                                me={me}
                                entries={sections.debug}
                                group={t.nav.groupDebug}
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
                        <MaintenanceBanner />
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

/** What is left between the rail's edge and an entry scrolled back into sight. */
const ENTRY_MARGIN = 8;

/**
 * Keeps the current page's entry in sight: with every group open, a short window's rail scrolls,
 * and a page reached from the search or a link may have its entry out of it.
 */
const useCurrentEntryInSight = () => {
    const nav = useRef<HTMLElement>(null);
    const router = useRouter();
    useEffect(() => {
        const place = () => {
            const list = nav.current;
            const entry = list?.querySelector('[aria-current="page"]');
            if (list === null || entry === null || entry === undefined) {
                return;
            }
            const [box, row] = [list.getBoundingClientRect(), entry.getBoundingClientRect()];
            if (row.top < box.top) {
                list.scrollTop -= box.top - row.top + ENTRY_MARGIN;
            } else if (row.bottom > box.bottom) {
                list.scrollTop += row.bottom - box.bottom + ENTRY_MARGIN;
            }
        };
        place();
        return router.subscribe('onRendered', place);
    }, [router]);
    return nav;
};

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
            {allowed.map(({ to, label, icon: Icon, alerts, waiting, nested }) => (
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
                    {waiting === true && <WaitingCount />}
                </Link>
            ))}
        </>
    );
}

/** Whether the Debug group was left open, kept in this browser. */
const FOLD_KEY = 'observer.navDebug';

const leftOpen = () => {
    try {
        return localStorage.getItem(FOLD_KEY) === 'open';
    } catch {
        return false;
    }
};

/**
 * A group that stays one line until it is opened: its pages are for the day something misbehaves.
 * It is open while one of them is the current page.
 */
function NavFold({
    me,
    entries,
    group,
    onNavigate,
}: {
    me: Me;
    entries: NavEntry[];
    group: string;
    onNavigate: () => void;
}) {
    const inside = useRouterState({
        select: (state) => entries.some((entry) => state.location.pathname.startsWith(entry.to)),
    });
    const [open, setOpen] = useState(leftOpen);
    const allowed = entries.filter((entry) => opensEntry(me, entry));
    if (allowed.length === 0) {
        return null;
    }
    const shown = open || inside;
    return (
        <>
            <button
                type="button"
                className="nav-group"
                aria-expanded={shown}
                onClick={() => {
                    setOpen(!shown);
                    try {
                        localStorage.setItem(FOLD_KEY, shown ? 'folded' : 'open');
                    } catch {
                        // Private browsing: the group folds again with the page.
                    }
                }}
            >
                <ChevronRight size={12} strokeWidth={2} aria-hidden="true" />
                {group}
            </button>
            {shown && <NavEntries me={me} entries={allowed} onNavigate={onNavigate} />}
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

/** The requests waiting on the Allowlist entry, while the allowlist refuses and there is any. */
function WaitingCount() {
    const { t, format } = useI18n();
    const allowlist = useAdmission()?.allowlist;
    if (allowlist === undefined || !allowlist.enabled || allowlist.waiting === 0) {
        return null;
    }
    return (
        <Chip tone="accent" className="ml-auto">
            <span aria-hidden="true">{format.number(allowlist.waiting)}</span>
            <span className="sr-only">{t.nav.waitingCount(allowlist.waiting)}</span>
        </Chip>
    );
}
