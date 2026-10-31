import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
    Outlet,
    RouterProvider,
} from '@tanstack/react-router';
import { act, cleanup, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Crash } from '../api/generated/Crash';
import type { Me } from '../api/generated/Me';
import { MeContext } from '../auth/me';
import { CRASH } from '../diagnostics/fixtures';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { CrashDrops } from './CrashDrops';
import { ANALYST, OWNER } from './fixtures';
import { rangeCovering } from './search';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** A diagnostics viewer without the drops. */
const DIAGNOSTICS: Me = { ...ANALYST, permissions: '1000' };

/** The line of `crash` for `me`, Observer answering that its run dropped 54 players. */
const line = async (me: Me, crash: Crash = CRASH) => {
    const fetch = vi.fn(async () => Response.json({ run: 40, players: 54 }));
    vi.stubGlobal('fetch', fetch);
    const root = createRootRoute({ component: Outlet });
    const router = createRouter({
        routeTree: root.addChildren([
            createRoute({
                getParentRoute: () => root,
                path: '/',
                component: () => <CrashDrops crash={crash} />,
            }),
        ]),
        history: createMemoryHistory({ initialEntries: ['/'] }),
    });
    render(
        <I18nProvider>
            <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
                <MeContext value={me}>
                    <RouterProvider router={router} />
                </MeContext>
            </QueryClientProvider>
        </I18nProvider>,
    );
    await settle();
    await settle();
    return fetch;
};

describe('CrashDrops', () => {
    beforeEach(() => {
        vi.stubGlobal('scrollTo', () => {});
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should say how many players a crash dropped, and link who sees players to their drops', async () => {
        const fetch = await line(OWNER);

        expect(screen.getByText(en.drops.crash.dropped(54))).not.toBeNull();
        expect(screen.getByRole('link', { name: en.drops.crash.showInLog }).getAttribute('href')).toBe(
            `/drops?range=${rangeCovering(CRASH.at, Date.now())}&run=40`,
        );
        expect(fetch).toHaveBeenCalledWith(
            `/api/servers/default/drops/crash?started=${encodeURIComponent(CRASH.runStarted!)}&generation=4`,
            expect.anything(),
        );
    });

    it('should give a statistics viewer the count without the way to the log', async () => {
        await line(ANALYST);

        expect(screen.getByText(en.drops.crash.dropped(54))).not.toBeNull();
        expect(screen.queryByRole('link')).toBeNull();
    });

    it('should ask nothing for who does not see drops, nor for dumps found without a run', async () => {
        expect(await line(DIAGNOSTICS)).not.toHaveBeenCalled();
        expect(screen.queryByText(en.drops.crash.dropped(54))).toBeNull();
        cleanup();

        expect(await line(OWNER, { ...CRASH, kind: 'found' })).not.toHaveBeenCalled();
    });
});
