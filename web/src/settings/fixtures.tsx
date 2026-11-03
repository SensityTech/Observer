import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
    Outlet,
    RouterProvider,
} from '@tanstack/react-router';
import { act, render } from '@testing-library/react';
import type { ReactNode } from 'react';

import type { Me } from '../api/generated/Me';
import { MeContext } from '../auth/me';
import { I18nProvider } from '../i18n/I18nProvider';
import { SettingsPage } from '../pages/SettingsPage';
import { SettingsCategory } from './Category';

export const OWNER: Me = {
    id: 1,
    username: 'ada',
    isOwner: true,
    permissions: 'fffff',
    mustChangePassword: false,
};

export const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/**
 * The Settings page opened at `path` under a router of its own, for `me`. `around` adds what the
 * shell provides above the pages, such as the live link.
 */
export const openSettings = async (
    path: string,
    {
        me = OWNER,
        client = new QueryClient({
            defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
        }),
        around = (page: ReactNode) => page,
    }: { me?: Me; client?: QueryClient; around?: (page: ReactNode) => ReactNode } = {},
) => {
    const root = createRootRoute({ component: Outlet });
    const settings = createRoute({ getParentRoute: () => root, path: '/settings', component: SettingsPage });
    const router = createRouter({
        routeTree: root.addChildren([
            settings.addChildren([
                createRoute({
                    getParentRoute: () => settings,
                    path: '$category',
                    component: SettingsCategory,
                }),
            ]),
        ]),
        history: createMemoryHistory({ initialEntries: [path] }),
    });
    const view = render(
        <I18nProvider>
            <QueryClientProvider client={client}>
                <MeContext value={me}>{around(<RouterProvider router={router} />)}</MeContext>
            </QueryClientProvider>
        </I18nProvider>,
    );
    await settle();
    await settle();
    return { ...view, client, router };
};
