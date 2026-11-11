import type { QueryClient } from '@tanstack/react-query';
import {
    createRootRouteWithContext,
    createRoute,
    createRouter,
    notFound,
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
import { parseStatisticsSearch, type StatisticsSearch } from './statistics/search';
import { lazyCode } from './lib/lazyCode';
import { LoginPage } from './pages/LoginPage';
import { NotFoundPage } from './pages/NotFoundPage';
import { OverviewPage } from './pages/OverviewPage';
import { PasswordChangePage } from './pages/PasswordChangePage';
import { RouteError } from './pages/RouteError';
import { SetupPage } from './pages/SetupPage';
import { firstCategory, isCategory, type CategoryId } from './settings/catalog';
import { AppShell } from './shell/AppShell';
import { FocusShell } from './shell/FocusShell';

// Pages beyond the Overview load when someone opens them: the statistics and their charts, the
// console's virtual list and colour parser, the restart schedule, the diagnostics, the players and
// their drops, the changes and their diffs, server setup and server tools, the settings and the
// audit log are no one's landing page.
const ConsolePage = lazyCode(() =>
    import('./pages/ConsolePage').then((page) => ({ default: page.ConsolePage })),
);
const ResourcesPage = lazyCode(() =>
    import('./pages/ResourcesPage').then((page) => ({ default: page.ResourcesPage })),
);
const SchedulePage = lazyCode(() =>
    import('./pages/SchedulePage').then((page) => ({ default: page.SchedulePage })),
);
const ArtifactsPage = lazyCode(() =>
    import('./pages/ArtifactsPage').then((page) => ({ default: page.ArtifactsPage })),
);
const ServerSetupPage = lazyCode(() =>
    import('./pages/ServerSetupPage').then((page) => ({ default: page.ServerSetupPage })),
);
const ToolsPage = lazyCode(() => import('./pages/ToolsPage').then((page) => ({ default: page.ToolsPage })));
const CfgPage = lazyCode(() => import('./pages/CfgPage').then((page) => ({ default: page.CfgPage })));
const ArchetypesPage = lazyCode(() =>
    import('./pages/ArchetypesPage').then((page) => ({ default: page.ArchetypesPage })),
);
const HeavyAssetsPage = lazyCode(() =>
    import('./pages/ToolWarningsPage').then((page) => ({
        default: () => <page.ToolWarningsPage kind="heavy" />,
    })),
);
const MissingFilesPage = lazyCode(() =>
    import('./pages/ToolWarningsPage').then((page) => ({
        default: () => <page.ToolWarningsPage kind="missing" />,
    })),
);
const BackupsPage = lazyCode(() =>
    import('./pages/BackupsPage').then((page) => ({ default: page.BackupsPage })),
);
const AuditPage = lazyCode(() => import('./pages/AuditPage').then((page) => ({ default: page.AuditPage })));
const AboutPage = lazyCode(() => import('./pages/AboutPage').then((page) => ({ default: page.AboutPage })));
const DiagnosticsPage = lazyCode(() =>
    import('./pages/DiagnosticsPage').then((page) => ({ default: page.DiagnosticsPage })),
);
const DebugEventsPage = lazyCode(() =>
    import('./pages/DebugEventsPage').then((page) => ({ default: page.DebugEventsPage })),
);
const DebugBagsPage = lazyCode(() =>
    import('./pages/DebugBagsPage').then((page) => ({ default: page.DebugBagsPage })),
);
const DebugHttpPage = lazyCode(() =>
    import('./pages/DebugHttpPage').then((page) => ({ default: page.DebugHttpPage })),
);
const PlayersPage = lazyCode(() =>
    import('./pages/PlayersPage').then((page) => ({ default: page.PlayersPage })),
);
const BansPage = lazyCode(() => import('./pages/BansPage').then((page) => ({ default: page.BansPage })));
const MapPage = lazyCode(() => import('./pages/MapPage').then((page) => ({ default: page.MapPage })));
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
const StatisticsPage = lazyCode(() =>
    import('./pages/StatisticsPage').then((page) => ({ default: page.StatisticsPage })),
);
const SettingsPage = lazyCode(() =>
    import('./pages/SettingsPage').then((page) => ({ default: page.SettingsPage })),
);
const SettingsCategory = lazyCode(() =>
    import('./settings/Category').then((page) => ({ default: page.SettingsCategory })),
);

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

/** The account a signed-in screen opens for, or where a visitor without one belongs. */
const signedIn = async ({ context: { queryClient } }: { context: RouterContext }) => {
    const me = await queryClient.ensureQueryData(meQuery);
    if (me === null) {
        throw await signedOut(queryClient);
    }
    // The API refuses everything else until then; the panel goes straight to the form.
    if (me.mustChangePassword) {
        throw redirect({ to: '/password' });
    }
    return { me };
};

const panelRoute = createRoute({
    getParentRoute: () => rootRoute,
    id: 'panel',
    beforeLoad: signedIn,
    component: AppShell,
});

/** Flows on the whole screen, without the panel's rail and columns. */
const focusRoute = createRoute({
    getParentRoute: () => rootRoute,
    id: 'focus',
    beforeLoad: signedIn,
    component: FocusShell,
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

// The tab, the ranges, the zoom and the lanes are in the address, so that a view is shared by its link.
const statisticsRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/statistics',
    validateSearch: (search: Record<string, unknown>): StatisticsSearch => parseStatisticsSearch(search),
    component: () => (
        <RequirePermission name="DashboardView">
            <StatisticsPage />
        </RequirePermission>
    ),
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

const resourcesRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/resources',
    component: () => (
        <RequirePermission name={['DashboardView', 'ResourcesControl']}>
            <ResourcesPage />
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

// What the resources count reads like the diagnostics: the same permission opens both.
const debugEventsRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/debug/events',
    component: () => (
        <RequirePermission name="DiagnosticsView">
            <DebugEventsPage />
        </RequirePermission>
    ),
});

const debugBagsRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/debug/state-bags',
    component: () => (
        <RequirePermission name="DiagnosticsView">
            <DebugBagsPage />
        </RequirePermission>
    ),
});

const debugHttpRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/debug/http',
    component: () => (
        <RequirePermission name="DiagnosticsView">
            <DebugHttpPage />
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

const mapRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/map',
    component: () => (
        <RequirePermission name="PlayersMap">
            <MapPage />
        </RequirePermission>
    ),
});
const bansRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/bans',
    component: () => (
        <RequirePermission name="BansView">
            <BansPage />
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

// Launch settings are the owner's alone; the setup is a flow of its own, without a page to refuse
// from, so anyone else goes back to where they land.
const serverSetupRoute = createRoute({
    getParentRoute: () => focusRoute,
    path: '/server/setup',
    beforeLoad: ({ context: { me } }) => {
        if (!me.isOwner) {
            throw redirect({ to: landing(me) });
        }
    },
    component: ServerSetupPage,
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

const archetypesRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/tools/duplicate-archetypes',
    component: () => (
        <RequireOwner>
            <ArchetypesPage />
        </RequireOwner>
    ),
});

const heavyAssetsRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/tools/heavy-assets',
    component: () => (
        <RequireOwner>
            <HeavyAssetsPage />
        </RequireOwner>
    ),
});

const missingFilesRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/tools/missing-files',
    component: () => (
        <RequireOwner>
            <MissingFilesPage />
        </RequireOwner>
    ),
});

// The cfg holds the server's secrets and decides what it runs: the owner's, as the launch settings are.
const cfgRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/cfg',
    component: () => (
        <RequireOwner>
            <CfgPage />
        </RequireOwner>
    ),
});

const backupsRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/backups',
    component: () => (
        <RequirePermission name="BackupsView">
            <BackupsPage />
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

// Every category guards itself, as the pages it took in did: the API refuses the rest anyway.
const settingsRoute = createRoute({
    getParentRoute: () => panelRoute,
    path: '/settings',
    component: SettingsPage,
});

const settingsIndexRoute = createRoute({
    getParentRoute: () => settingsRoute,
    path: '/',
    beforeLoad: ({ context: { me } }) => {
        throw redirect({ to: '/settings/$category', params: { category: firstCategory(me) }, replace: true });
    },
});

const settingsCategoryRoute = createRoute({
    getParentRoute: () => settingsRoute,
    path: '$category',
    beforeLoad: ({ params }) => {
        if (!isCategory(params.category)) {
            throw notFound();
        }
    },
    component: SettingsCategory,
});

/** A page that became a category of the settings: its address, bookmarked or linked, leads there. */
const moved = <Path extends string>(path: Path, category: CategoryId) =>
    createRoute({
        getParentRoute: () => panelRoute,
        path,
        beforeLoad: ({ location }) => {
            throw redirect({
                to: '/settings/$category',
                params: { category },
                hash: location.hash === '' ? undefined : location.hash,
                replace: true,
            });
        },
    });

const serverRoute = moved('/server', 'launch');
const usersRoute = moved('/users', 'users');
const filesRoute = moved('/files', 'files');
const accountRoute = moved('/account', 'account');

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
        statisticsRoute,
        consoleRoute,
        resourcesRoute,
        scheduleRoute,
        artifactsRoute,
        diagnosticsRoute,
        debugEventsRoute,
        debugBagsRoute,
        debugHttpRoute,
        playersRoute,
        playerRoute,
        mapRoute,
        bansRoute,
        changesRoute,
        changesResourceRoute,
        dropsRoute,
        serverRoute,
        toolsRoute,
        archetypesRoute,
        heavyAssetsRoute,
        missingFilesRoute,
        cfgRoute,
        backupsRoute,
        usersRoute,
        filesRoute,
        auditRoute,
        settingsRoute.addChildren([settingsIndexRoute, settingsCategoryRoute]),
        accountRoute,
        aboutRoute,
    ]),
    focusRoute.addChildren([serverSetupRoute]),
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
