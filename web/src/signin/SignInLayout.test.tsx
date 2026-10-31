/// <reference types="vitest/jsdom" />
import { createMemoryHistory, createRootRoute, createRouter, RouterProvider } from '@tanstack/react-router';
import { cleanup, render, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { SignInLayout } from './SignInLayout';

/** The sign-in page's layout around an empty form. */
const signIn = () => {
    const root = createRootRoute({
        component: () => (
            <SignInLayout
                eyebrow={en.login.eyebrow}
                title={en.login.title}
                tone={en.login.tone}
                lead={en.login.lead}
            >
                <form />
            </SignInLayout>
        ),
    });
    const router = createRouter({ routeTree: root, history: createMemoryHistory({ initialEntries: ['/'] }) });
    return render(
        <I18nProvider>
            <RouterProvider router={router} />
        </I18nProvider>,
    );
};

describe('SignInLayout', () => {
    beforeEach(() => {
        // Each navigation scrolls to the top, which jsdom does not implement.
        vi.stubGlobal('scrollTo', () => {});
        // The stage stays still, as under reduced motion: jsdom draws no canvas.
        vi.stubGlobal('matchMedia', () => ({
            matches: true,
            addEventListener: () => {},
            removeEventListener: () => {},
        }));
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        jsdom.reconfigure({ url: 'http://localhost:3000/' });
    });

    it('should warn on the pages before sign-in when the panel is reached over plain HTTP at a LAN address', async () => {
        jsdom.reconfigure({ url: 'http://192.168.1.20:40125/' });

        const { findByRole } = signIn();

        expect(within(await findByRole('main')).getByRole('alert').textContent).toBe(
            `${en.app.plainHttp.title} ${en.app.plainHttp.body}`,
        );
    });
});
