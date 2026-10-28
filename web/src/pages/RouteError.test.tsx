import {
    createMemoryHistory,
    createRootRoute,
    createRoute,
    createRouter,
    Outlet,
    RouterProvider,
} from '@tanstack/react-router';
import { cleanup, render } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { lazyCode } from '../lib/lazyCode';
import { RouteError } from './RouteError';

/** A panel whose only page fails as `page` does. */
const panel = (page: () => ReactNode) => {
    const root = createRootRoute({ component: Outlet, errorComponent: RouteError });
    const index = createRoute({ getParentRoute: () => root, path: '/', component: page });
    const router = createRouter({
        routeTree: root.addChildren([index]),
        history: createMemoryHistory({ initialEntries: ['/'] }),
    });
    return render(
        <I18nProvider>
            <RouterProvider router={router} />
        </I18nProvider>,
    );
};

describe('RouteError', () => {
    afterEach(() => {
        cleanup();
        vi.restoreAllMocks();
    });

    it('should ask for a reload when the code of a page could not be loaded', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {});
        const Console = lazyCode(() =>
            Promise.reject(
                new TypeError('Failed to fetch dynamically imported module: /assets/ConsolePage.js'),
            ),
        );

        const { findByText, getByRole } = panel(() => <Console />);

        expect(await findByText(en.failure.code)).not.toBeNull();
        expect(getByRole('button').textContent).toBe(en.common.reload);
    });

    it('should still say Observer cannot be reached when a request got no answer', async () => {
        vi.spyOn(console, 'error').mockImplementation(() => {});

        const { findByText, getByRole } = panel(() => {
            throw new TypeError('Failed to fetch');
        });

        expect(await findByText(en.failure.network)).not.toBeNull();
        expect(getByRole('button').textContent).toBe(en.common.retry);
    });
});
