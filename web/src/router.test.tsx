import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { meQuery, setupQuery } from './api/auth';
import type { Me } from './api/generated/Me';
import type { StoredLaunch } from './api/generated/StoredLaunch';
import { DEFAULT_LAUNCH, launchQuery } from './api/launch';
import { en } from './i18n/en';
import { I18nProvider } from './i18n/I18nProvider';
import { createPanelRouter } from './router';

// The shell and the pages stand in by their headings: what is tested is where the panel goes.
vi.mock('./shell/AppShell', async () => {
    const { Outlet } = await import('@tanstack/react-router');
    const { useQuery } = await import('@tanstack/react-query');
    const { MeContext } = await import('./auth/me');
    const auth = await import('./api/auth');
    return {
        AppShell: () => {
            const { data: me } = useQuery(auth.meQuery);
            return me ? (
                <MeContext value={me}>
                    <Outlet />
                </MeContext>
            ) : null;
        },
    };
});
vi.mock('./pages/OverviewPage', () => ({ OverviewPage: () => <h1>Overview</h1> }));
vi.mock('./pages/ServerSetupPage', () => ({ ServerSetupPage: () => <h1>Server setup</h1> }));
vi.mock('./pages/SettingsPage', async () => {
    const { Outlet } = await import('@tanstack/react-router');
    return { SettingsPage: () => <Outlet /> };
});
vi.mock('./settings/Category', async () => {
    const { useParams } = await import('@tanstack/react-router');
    return {
        SettingsCategory: () => {
            const { category } = useParams({ strict: false });
            return <h1>Settings {category}</h1>;
        },
    };
});

const OWNER: Me = { id: 1, username: 'ada', isOwner: true, permissions: 'fff', mustChangePassword: false };
/** A moderator who views the dashboard and nothing else. */
const VIEWER: Me = { id: 2, username: 'kenji', isOwner: false, permissions: '1', mustChangePassword: false };
/** An account that holds no permission: its own settings are all it opens. */
const NOBODY: Me = { id: 3, username: 'lin', isOwner: false, permissions: '0', mustChangePassword: false };

const SAVED: StoredLaunch = {
    settings: { ...DEFAULT_LAUNCH, dataPath: 'D:\\FXServer\\server-data' },
    version: 1,
    legacyBuildPath: null,
};

/** The panel at `path`, its queries already answered as `cached` says. */
const panel = (path: string, cached: [readonly unknown[], unknown][]) => {
    window.history.replaceState(null, '', path);
    const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity, retry: false } } });
    for (const [key, data] of cached) {
        client.setQueryData(key, data);
    }
    const router = createPanelRouter(client);
    const view = render(
        <I18nProvider>
            <QueryClientProvider client={client}>
                <RouterProvider router={router} />
            </QueryClientProvider>
        </I18nProvider>,
    );
    return { ...view, router };
};

describe('the panel router', () => {
    beforeEach(() => {
        // Each navigation scrolls to the top, which jsdom does not implement.
        vi.stubGlobal('scrollTo', () => {});
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        localStorage.clear();
    });

    it('should send the owner from the Overview to the server setup while nothing is saved', async () => {
        const { findByRole, router } = panel('/', [
            [meQuery.queryKey, OWNER],
            [launchQuery.queryKey, null],
        ]);

        await findByRole('heading', { name: 'Server setup' });
        expect(router.state.location.pathname).toBe('/server/setup');
    });

    it('should land the owner on the Overview once the launch settings are saved', async () => {
        const { findByRole, router } = panel('/', [
            [meQuery.queryKey, OWNER],
            [launchQuery.queryKey, SAVED],
        ]);

        await findByRole('heading', { name: 'Overview' });
        expect(router.state.location.pathname).toBe('/');
    });

    it('should land another account on the Overview without asking for the launch settings', async () => {
        const fetch = vi.fn(async () => Response.json({ code: 'owner_only' }, { status: 403 }));
        vi.stubGlobal('fetch', fetch);

        const { findByRole } = panel('/', [[meQuery.queryKey, VIEWER]]);

        await findByRole('heading', { name: 'Overview' });
        expect(fetch).not.toHaveBeenCalled();
    });

    it.each([
        ['/server', 'launch', ''],
        ['/server#setting-hangDetection', 'launch', 'setting-hangDetection'],
        ['/users', 'users', ''],
        ['/files', 'files', ''],
        ['/account', 'account', ''],
    ])(
        'should take %s, a page that became settings, to its category, its hash kept',
        async (path, category, hash) => {
            const { findByRole, router } = panel(path, [[meQuery.queryKey, OWNER]]);

            await findByRole('heading', { name: `Settings ${category}` });
            expect(router.state.location.pathname).toBe(`/settings/${category}`);
            expect(router.state.location.hash).toBe(hash);
        },
    );

    it.each([
        [OWNER, 'launch'],
        [VIEWER, 'restarts'],
        [NOBODY, 'account'],
    ] as const)('should open the settings of %o on the first category it may see', async (me, category) => {
        const { findByRole, router } = panel('/settings', [[meQuery.queryKey, me]]);

        await findByRole('heading', { name: `Settings ${category}` });
        expect(router.state.location.pathname).toBe(`/settings/${category}`);
    });

    it('should say a category of the settings that does not exist is not found', async () => {
        const { findByText, queryByRole } = panel('/settings/everything', [[meQuery.queryKey, OWNER]]);

        await findByText(en.notFound.title);
        expect(queryByRole('heading', { name: /^Settings/ })).toBeNull();
    });

    it('should take the new owner from the account setup to the server setup', async () => {
        // The sign-in stage stays still, as under reduced motion: jsdom draws no canvas.
        vi.stubGlobal('matchMedia', () => ({
            matches: true,
            addEventListener: () => {},
            removeEventListener: () => {},
        }));
        vi.stubGlobal(
            'fetch',
            vi.fn(async () => Response.json({ sessionKey: 'key', me: OWNER })),
        );
        const { findByRole, getByLabelText, getByRole, router } = panel('/setup#TOKEN', [
            [setupQuery.queryKey, { required: true }],
            [meQuery.queryKey, null],
        ]);
        await findByRole('button', { name: en.setup.submit });

        fireEvent.change(getByLabelText(en.setup.username), { target: { value: 'ada' } });
        fireEvent.change(getByLabelText(en.setup.password), { target: { value: 'a long owner passphrase' } });
        fireEvent.change(getByLabelText(en.setup.confirm), { target: { value: 'a long owner passphrase' } });
        fireEvent.click(getByRole('button', { name: en.setup.submit }));

        await findByRole('heading', { name: 'Server setup' });
        await waitFor(() => expect(router.state.location.pathname).toBe('/server/setup'));
    });
});
