import type { QueryClient } from '@tanstack/react-query';
import {
    createRootRouteWithContext,
    createRoute,
    createRouter,
    Outlet,
    redirect,
} from '@tanstack/react-router';

import { meQuery, setupQuery } from './api/auth';
import { Permission } from './api/generated/Permission';
import { allows, landing } from './auth/me';
import { ReauthDialog } from './auth/ReauthDialog';
import { RequireOwner, RequirePermission } from './auth/RequirePermission';
import { AccountPage } from './pages/AccountPage';
import { AuditPage } from './pages/AuditPage';
import { ConsolePage } from './pages/ConsolePage';
import { LoginPage } from './pages/LoginPage';
import { NotFoundPage } from './pages/NotFoundPage';
import { OverviewPage } from './pages/OverviewPage';
import { PasswordChangePage } from './pages/PasswordChangePage';
import { RouteError } from './pages/RouteError';
import { ServerPage } from './pages/ServerPage';
import { SetupPage } from './pages/SetupPage';
import { UsersPage } from './pages/UsersPage';
import { AppShell } from './shell/AppShell';

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
