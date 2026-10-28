import { Link, Outlet } from '@tanstack/react-router';
import { LayoutDashboard, Menu, X } from 'lucide-react';
import { useEffect, useState } from 'react';

import { useI18n } from '../i18n/I18nProvider';
import { Logo } from './Logo';
import { StatusBar } from './StatusBar';
import { ThemeSwitch } from './ThemeSwitch';

export function AppShell() {
    const { t } = useI18n();
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

    return (
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
                            className="btn btn-ghost btn-icon rail-close"
                            aria-label={t.nav.close}
                            onClick={closeMenu}
                        >
                            <X size={16} strokeWidth={1.5} aria-hidden="true" />
                        </button>
                    </div>
                    <nav className="nav" aria-label={t.nav.label}>
                        <Link to="/" className="nav-item" activeOptions={{ exact: true }} onClick={closeMenu}>
                            <LayoutDashboard size={16} strokeWidth={1.5} aria-hidden="true" />
                            {t.nav.overview}
                        </Link>
                    </nav>
                    <div className="rail-foot">
                        <ThemeSwitch />
                    </div>
                </div>
            </aside>
            {menuOpen && <div className="scrim" aria-hidden="true" onClick={closeMenu} />}

            <div className="main" inert={menuOpen}>
                <div className="mobilebar">
                    <button
                        type="button"
                        className="btn btn-ghost btn-icon"
                        aria-label={t.nav.open}
                        aria-controls="rail"
                        aria-expanded={menuOpen}
                        onClick={() => setMenuOpen(true)}
                    >
                        <Menu size={18} strokeWidth={1.5} aria-hidden="true" />
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
    );
}
