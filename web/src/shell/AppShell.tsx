import { useQuery } from '@tanstack/react-query';
import { Link, Outlet } from '@tanstack/react-router';
import { LayoutDashboard, Menu as MenuIcon, Users, X, type LucideIcon } from 'lucide-react';
import { useEffect, useState } from 'react';

import { meQuery } from '../api/auth';
import { Permission } from '../api/generated/Permission';
import type { Me } from '../api/generated/Me';
import { allows, MeContext } from '../auth/me';
import { buttonVariants } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { Logo } from './Logo';
import { StatusBar } from './StatusBar';
import { ThemeSwitch } from './ThemeSwitch';
import { UserMenu } from './UserMenu';

type NavEntry = { to: '/' | '/users'; label: string; icon: LucideIcon; permission: bigint };

export function AppShell() {
    const { t } = useI18n();
    const { data: me } = useQuery(meQuery);
    const [menuOpen, setMenuOpen] = useState(false);
    const closeMenu = () => setMenuOpen(false);

    useEffect(() => {
        document.documentElement.classList.toggle('drawer-open', menuOpen);
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

    const main: NavEntry[] = [
        { to: '/', label: t.nav.overview, icon: LayoutDashboard, permission: Permission.DashboardView },
    ];
    const administration: NavEntry[] = [
        { to: '/users', label: t.nav.users, icon: Users, permission: Permission.UsersManage },
    ];

    return (
        <MeContext value={me}>
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
                    <header className="topbar">
                        <StatusBar />
                    </header>
                    <main className="content" id="content" tabIndex={-1}>
                        <Outlet />
                    </main>
                </div>
            </div>
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
    const allowed = entries.filter((entry) => allows(me, entry.permission));
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
