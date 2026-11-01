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
import { launchQuery } from './api/launch';
import { allows, landing } from './auth/me';
import { ReauthDialog } from './auth/ReauthDialog';
import { RequireOwner, RequirePermission } from './auth/RequirePermission';
import { changesSearch, resourceSearch } from './changes/search';
import { parseDropsSearch, type DropsSearch } from './drops/search';
import { lazyCode } from './lib/lazyCode';
import { AccountPage } from './pages/AccountPage';
import { LoginPage } from './pages/LoginPage';
import { NotFoundPage } from './pages/NotFoundPage';
import { OverviewPage } from './pages/OverviewPage';
import { PasswordChangePage } from './pages/PasswordChangePage';
import { RouteError } from './pages/RouteError';
import { SetupPage } from './pages/SetupPage';
import { AppShell } from './shell/AppShell';

// Pages beyond the Overview load when someone opens them: the console's virtual list and colour
// parser, the restart schedule, the diagnostics, the players and their drops, the changes and their
// diffs, the owner's launch settings, server setup and server tools, the accounts and the audit log
// are no one's landing page.
const ConsolePage = lazyCode(() =>
    import('./pages/ConsolePage').then((page) => ({ default: page.ConsolePage })),
);
const SchedulePage = lazyCode(() =>
    import('./pages/SchedulePage').then((page) => ({ default: page.SchedulePage })),
);
const ArtifactsPage = lazyCode(() =>
    import('./pages/ArtifactsPage').then((page) => ({ default: page.ArtifactsPage })),
);
const ServerPage = lazyCode(() =>
    import('./pages/ServerPage').then((page) => ({ default: page.ServerPage })),
);
const ServerSetupPage = lazyCode(() =>
    import('./pages/ServerSetupPage').then((page) => ({ default: page.ServerSetupPage })),
);
const ToolsPage = lazyCode(() => import('./pages/ToolsPage').then((page) => ({ default: page.ToolsPage })));
const UsersPage = lazyCode(() => import('./pages/UsersPage').then((page) => ({ default: page.UsersPage })));
const FilesPage = lazyCode(() => import('./pages/FilesPage').then((page) => ({ default: page.FilesPage })));
const AuditPage = lazyCode(() => import('./pages/AuditPage').then((page) => ({ default: page.AuditPage })));
const AboutPage = lazyCode(() => import('./pages/AboutPage').then((page) => ({ default: page.AboutPage })));
const DiagnosticsPage = lazyCode(() =>
    import('./pages/DiagnosticsPage').then((page) => ({ default: page.DiagnosticsPage })),
);
const PlayersPage = lazyCode(() =>
    import('./pages/PlayersPage').then((page) => ({ default: page.PlayersPage })),
);
const PlayerPage = lazyCode(() =>
    import('./pages/PlayerPage').then((page) => ({ default: page.PlayerPage })),
);
const ChangesPage = lazyCode(() =>
    import('./pages/ChangesPage').then((page) => ({ default: page.ChangesPage })),
);
const ChangesResourcePage = lazyCode(() =>
    import('./pages/ChangesResourcePage').then((page) => ({ default: page.ChangesResourcePage })),
);
const DropsPage = lazyCode(() => import('./pages/DropsPage').then((page) => ({ default: page.DropsPage })));

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
    beforeLoad: async ({ context: { me, queryClient } }) => {
        if (!allows(me, Permission.DashboardView)) {
            throw redirect({ to: landing(me) });
        }
        // Until the server is set up, the owner lands on its setup. Settings that cannot be read
        // leave the Overview to say so.
        if (me.isOwner && (await queryClient.ensureQueryData(launchQuery).catch(() => undefined)) === null) {
            throw redirect({ to: '/server/setup' });
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

const scheduleRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/schedule',
    component: () => (
        <RequirePermission name={['DashboardView', 'ScheduleManage', 'ServerControl']}>
            <SchedulePage />
        </RequirePermission>
    ),
});

const artifactsRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/artifacts',
    component: () => (
        <RequirePermission name="ArtifactsView">
            <ArtifactsPage />
        </RequirePermission>
    ),
});

const diagnosticsRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/diagnostics',
    component: () => (
        <RequirePermission name="DiagnosticsView">
            <DiagnosticsPage />
        </RequirePermission>
    ),
});

const playersRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/players',
    component: () => (
        <RequirePermission name="PlayersView">
            <PlayersPage />
        </RequirePermission>
    ),
});

const playerRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/players/$id',
    component: () => (
        <RequirePermission name="PlayersView">
            <PlayerPage />
        </RequirePermission>
    ),
});

// The range, the chart and the log's filters are in the address, so other pages link to a narrowed
// view: the profile to a player's drops, Diagnostics to a crash's.
const dropsRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/drops',
    validateSearch: (search: Record<string, unknown>): DropsSearch => parseDropsSearch(search),
    component: () => (
        <RequirePermission name="DropsView">
            <DropsPage />
        </RequirePermission>
    ),
});

const changesRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/changes',
    validateSearch: changesSearch,
    component: () => (
        <RequirePermission name="ChangesView">
            <ChangesPage />
        </RequirePermission>
    ),
});

const changesResourceRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/changes/resource',
    validateSearch: resourceSearch,
    component: () => (
        <RequirePermission name="ChangesView">
            <ChangesResourcePage />
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

const serverSetupRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/server/setup',
    component: () => (
        <RequireOwner>
            <ServerSetupPage />
        </RequireOwner>
    ),
});

// The tools read the whole server folder and write into it: the owner's, as the server setup is.
const toolsRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/tools',
    component: () => (
        <RequireOwner>
            <ToolsPage />
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

const filesRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/files',
    component: () => (
        <RequirePermission name="FilesManage">
            <FilesPage />
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

// Any signed-in account may read it: only strangers are kept from the list of what Observer runs.
const aboutRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/about',
    component: AboutPage,
});

const routeTree = rootRoute.addChildren([
    setupRoute,
    loginRoute,
    passwordRoute,
    panelRoute.addChildren([
        overviewRoute,
        consoleRoute,
        scheduleRoute,
        artifactsRoute,
        diagnosticsRoute,
        playersRoute,
        playerRoute,
        changesRoute,
        changesResourceRoute,
        dropsRoute,
        serverRoute,
        serverSetupRoute,
        toolsRoute,
        usersRoute,
        filesRoute,
        auditRoute,
        accountRoute,
        aboutRoute,
    ]),
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
    interface HistoryState {
        /** Set on the way back to the players list from a profile just forgotten. */
        forgotten?: boolean;
    }
}
