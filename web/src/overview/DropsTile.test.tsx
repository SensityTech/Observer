import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
    Outlet,
    RouterProvider,
} from '@tanstack/react-router';
import { act, cleanup, render, screen, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Me } from '../api/generated/Me';
import { MeContext } from '../auth/me';
import { ANALYST, SUMMARY_DAY } from '../drops/fixtures';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { DropsTile } from './Tiles';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** A viewer of the dashboard without the drops. */
const VIEWER: Me = { ...ANALYST, permissions: '1' };

/** The tile under a router of its own, for `me`, with Observer answering `answer`. */
const tile = async (me: Me, answer: () => Promise<Response>) => {
    const fetch = vi.fn(answer);
    vi.stubGlobal('fetch', fetch);
    const root = createRootRoute({ component: Outlet });
    const router = createRouter({
        routeTree: root.addChildren([
            createRoute({ getParentRoute: () => root, path: '/', component: DropsTile }),
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
    return fetch;
};

describe('the Overview tile of player drops', () => {
    beforeEach(() => {
        vi.stubGlobal('scrollTo', () => {});
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it("should show the last 24 hours' unexpected rate and its verdict to who sees drops", async () => {
        const fetch = await tile(ANALYST, async () => Response.json(SUMMARY_DAY));
        await settle();

        const drops = within(screen.getByRole('region', { name: en.drops.overview.label }));
        expect(drops.getByText('9.4').textContent).toBe(`9.4${en.drops.unit}`);
        expect(drops.getByRole('button', { name: en.drops.verdicts.noClearChange })).not.toBeNull();
        expect(drops.getByRole('link', { name: en.drops.overview.open }).getAttribute('href')).toBe(
            '/drops?range=24h',
        );
        expect(fetch).toHaveBeenCalledWith('/api/servers/default/drops/summary?range=24h', expect.anything());
    });

    it('should keep its place while it reads, and say when it could not', async () => {
        await tile(ANALYST, () => new Promise(() => {}));
        expect(screen.getByRole('region', { name: en.drops.overview.label }).getAttribute('aria-busy')).toBe(
            'true',
        );
        expect(screen.queryByText(en.drops.unknown)).toBeNull();
        cleanup();

        await tile(ANALYST, async () => Response.json({ code: 'internal', params: {} }, { status: 500 }));
        await settle();
        const drops = within(screen.getByRole('region', { name: en.drops.overview.label }));
        expect(drops.getByText(en.drops.overview.failed)).not.toBeNull();
    });

    it('should not be there, nor read anything, for who does not see drops', async () => {
        const fetch = await tile(VIEWER, async () => Response.json(SUMMARY_DAY));

        expect(screen.queryByRole('region', { name: en.drops.overview.label })).toBeNull();
        expect(fetch).not.toHaveBeenCalled();
    });
});
