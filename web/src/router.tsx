import { createRootRoute, createRoute, createRouter } from '@tanstack/react-router';

import { NotFoundPage } from './pages/NotFoundPage';
import { OverviewPage } from './pages/OverviewPage';
import { AppShell } from './shell/AppShell';

const rootRoute = createRootRoute({
    component: AppShell,
    notFoundComponent: NotFoundPage,
});

const overviewRoute = createRoute({
    getParentRoute: () => rootRoute,
    path: '/',
    component: OverviewPage,
});

export const router = createRouter({
    routeTree: rootRoute.addChildren([overviewRoute]),
    defaultPreload: 'intent',
});

declare module '@tanstack/react-router' {
    interface Register {
        router: typeof router;
    }
}
