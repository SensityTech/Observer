/// <reference types="vitest/jsdom" />
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
    RouterProvider,
} from '@tanstack/react-router';
import { cleanup, render, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { meQuery } from '../api/auth';
import { onlineQuery } from '../api/players';
import type { Me } from '../api/generated/Me';
import { Permission } from '../api/generated/Permission';
import { formatPermissions } from '../auth/permissions';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime } from '../live/realtime';
import { AppShell } from './AppShell';

const OWNER: Me = { id: 1, username: 'ada', isOwner: true, permissions: 'fff', mustChangePassword: false };

/** The signed-in shell around an Overview that stands in by its heading, its socket never answering. */
const shell = (me: Me = OWNER) => {
    const root = createRootRoute({ component: AppShell });
    const overview = createRoute({
        getParentRoute: () => root,
        path: '/',
        component: () => <h1>Overview</h1>,
    });
    const router = createRouter({
        routeTree: root.addChildren([overview]),
        history: createMemoryHistory({ initialEntries: ['/'] }),
    });
    const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity, retry: false } } });
    client.setQueryData(meQuery.queryKey, me);
    client.setQueryData(onlineQuery.queryKey, { players: [] });
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: () => ({ send: () => {}, close: () => {} }),
        refetch: () => {},
    });
    return render(
        <I18nProvider>
            <QueryClientProvider client={client}>
                <LiveContext value={live}>
                    <RouterProvider router={router} />
                </LiveContext>
            </QueryClientProvider>
        </I18nProvider>,
    );
};

describe('AppShell', () => {
    beforeEach(() => {
        // Each navigation scrolls to the top, which jsdom does not implement.
        vi.stubGlobal('scrollTo', () => {});
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        jsdom.reconfigure({ url: 'http://localhost:3000/' });
    });

    it('should warn on every page when the panel is reached over plain HTTP at a LAN address', async () => {
        jsdom.reconfigure({ url: 'http://192.168.1.20:40125/' });

        const { findByRole, getByRole } = shell();
        await findByRole('heading', { name: 'Overview' });

        expect(within(getByRole('main')).getByRole('alert').textContent).toBe(
            `${en.app.plainHttp.title} ${en.app.plainHttp.body}`,
        );
    });

    it('should show the players beside every page only to an account that sees players', async () => {
        const without = shell({ ...OWNER, id: 2, username: 'grace', isOwner: false, permissions: '1' });
        await without.findByRole('heading', { name: 'Overview' });
        const hidden = without.queryByRole('button', { name: en.online.show('0') });
        cleanup();

        const moderator = shell({ ...OWNER, id: 3, username: 'hopper', isOwner: false, permissions: '4001' });
        await moderator.findByRole('heading', { name: 'Overview' });

        expect(hidden).toBeNull();
        expect(moderator.getByRole('button', { name: en.online.show('0') })).not.toBeNull();
    });

    it('should list the sections by what one comes for: a player, the server, what it did, its upkeep', async () => {
        const every = Object.values(Permission).reduce((mask, permission) => mask | permission, 0n);
        const { findByRole, getByRole } = shell({ ...OWNER, permissions: formatPermissions(every) });
        await findByRole('heading', { name: 'Overview' });

        const nav = getByRole('navigation', { name: en.nav.sections });
        expect(
            within(nav)
                .getAllByRole('link')
                .map((link) => link.textContent),
        ).toEqual([
            en.nav.overview,
            en.nav.console,
            en.nav.players,
            en.nav.map,
            en.nav.bans,
            en.nav.schedule,
            en.nav.artifacts,
            en.nav.backups,
            en.nav.changes,
            en.nav.statistics,
            en.nav.drops,
            en.nav.diagnostics,
            en.nav.tools,
            en.nav.audit,
        ]);
        expect([...nav.querySelectorAll('.nav-group')].map((group) => group.textContent)).toEqual([
            en.nav.groupPlayers,
            en.nav.groupServer,
            en.nav.groupInsights,
            en.nav.maintenance,
        ]);
    });

    it('should keep the settings at the foot of the rail, for every account', async () => {
        const { findByRole, getByRole } = shell({
            ...OWNER,
            id: 2,
            username: 'grace',
            isOwner: false,
            permissions: '1',
        });
        await findByRole('heading', { name: 'Overview' });

        const rail = within(getByRole('complementary', { name: en.nav.label }));
        const settings = rail.getByRole('link', { name: en.nav.settings });
        expect(settings.getAttribute('href')).toBe('/settings');
        expect(
            within(getByRole('navigation', { name: en.nav.sections })).queryByRole('link', {
                name: en.nav.settings,
            }),
        ).toBeNull();
    });
});
