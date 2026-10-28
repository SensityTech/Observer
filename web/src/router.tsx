import type { QueryClient } from '@tanstack/react-query';
import {
    createRootRouteWithContext,
    createRoute,
    createRouter,
    Outlet,
    redirect,
} from '@tanstack/react-router';
import { lazy } from 'react';

import { meQuery, setupQuery } from './api/auth';
import { Permission } from './api/generated/Permission';
import { allows, landing } from './auth/me';
import { ReauthDialog } from './auth/ReauthDialog';
import { RequireOwner, RequirePermission } from './auth/RequirePermission';
import { AccountPage } from './pages/AccountPage';
import { LoginPage } from './pages/LoginPage';
import { NotFoundPage } from './pages/NotFoundPage';
import { OverviewPage } from './pages/OverviewPage';
import { PasswordChangePage } from './pages/PasswordChangePage';
import { RouteError } from './pages/RouteError';
import { SetupPage } from './pages/SetupPage';
import { AppShell } from './shell/AppShell';

// Pages beyond the Overview load when someone opens them: the console's virtual list and colour
// parser, the owner's launch settings, the accounts and the audit log are no one's landing page.
const ConsolePage = lazy(() => import('./pages/ConsolePage').then((page) => ({ default: page.ConsolePage })));
const ServerPage = lazy(() => import('./pages/ServerPage').then((page) => ({ default: page.ServerPage })));
const UsersPage = lazy(() => import('./pages/UsersPage').then((page) => ({ default: page.UsersPage })));
const AuditPage = lazy(() => import('./pages/AuditPage').then((page) => ({ default: page.AuditPage })));

type RouterContext = { queryClient: QueryClient };

/** Where a signed-out visitor belongs: the setup page until the owner exists, the sign-in after. */
const signedOut = async (queryClient: QueryClient) =>
    redirect({ to: (await queryClient.ensureQueryData(setupQuery)).required ? '/setup' : '/login' });

const rootRoute = createRootRouteWithContext<RouterContext>()({
    component: () => (
        <>
            <Outlet />
            <ReauthDialog />
        </>
    ),
    notFoundComponent: NotFoundPage,
    errorComponent: RouteError,
});

const setupRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/setup',
    beforeLoad: async ({ context: { queryClient } }) => {
        if (!(await queryClient.ensureQueryData(setupQuery)).required) {
            throw redirect({ to: '/login' });
        }
    },
    component: SetupPage,
});

const loginRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/login',
    beforeLoad: async ({ context: { queryClient } }) => {
        const me = await queryClient.ensureQueryData(meQuery);
        if (me !== null) {
            throw redirect({ to: me.mustChangePassword ? '/password' : landing(me) });
        }
        if ((await queryClient.ensureQueryData(setupQuery)).required) {
            throw redirect({ to: '/setup' });
        }
    },
    component: LoginPage,
});

const passwordRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/password',
    beforeLoad: async ({ context: { queryClient } }) => {
        const me = await queryClient.ensureQueryData(meQuery);
        if (me === null) {
            throw await signedOut(queryClient);
        }
        if (!me.mustChangePassword) {
            throw redirect({ to: landing(me) });
        }
    },
    component: PasswordChangePage,
});

const panelRoute = createRoute({
    getParentRoute: () => rootRoute,
    id: 'panel',
    beforeLoad: async ({ context: { queryClient } }) => {
        const me = await queryClient.ensureQueryData(meQuery);
        if (me === null) {
            throw await signedOut(queryClient);
        }
        // The API refuses everything else until then; the panel goes straight to the form.
        if (me.mustChangePassword) {
            throw redirect({ to: '/password' });
        }
        return { me };
    },
    component: AppShell,
});

const overviewRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/',
    beforeLoad: ({ context: { me } }) => {
        if (!allows(me, Permission.DashboardView)) {
            throw redirect({ to: landing(me) });
        }
    },
    component: OverviewPage,
});

const consoleRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/console',
    component: () => (
        <RequirePermission name="ConsoleView">
            <ConsolePage />
        </RequirePermission>
    ),
});

const serverRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/server',
    component: () => (
        <RequireOwner>
            <ServerPage />
        </RequireOwner>
    ),
});

const usersRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/users',
    component: () => (
        <RequirePermission name="UsersManage">
            <UsersPage />
        </RequirePermission>
    ),
});

const auditRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/audit',
    component: () => (
        <RequirePermission name="AuditView">
            <AuditPage />
        </RequirePermission>
    ),
});

const accountRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/account',
    component: AccountPage,
});

const routeTree = rootRoute.addChildren([
    setupRoute,
    loginRoute,
    passwordRoute,
    panelRoute.addChildren([overviewRoute, consoleRoute, serverRoute, usersRoute, auditRoute, accountRoute]),
]);

export const createPanelRouter = (queryClient: QueryClient) =>
    createRouter({
        routeTree,
        context: { queryClient },
        defaultPreload: 'intent',
        // The query cache already holds what the routes read.
        defaultPreloadStaleTime: 0,
    });

declare module '@tanstack/react-router' {
    interface Register {
        router: ReturnType<typeof createPanelRouter>;
    }
}
