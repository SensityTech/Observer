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
import type { Me } from '../api/generated/Me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime } from '../live/realtime';
import { AppShell } from './AppShell';

const OWNER: Me = { id: 1, username: 'ada', isOwner: true, permissions: 'fff', mustChangePassword: false };

/** The signed-in shell around an Overview that stands in by its heading, its socket never answering. */
const shell = () => {
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
    client.setQueryData(meQuery.queryKey, OWNER);
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
});
