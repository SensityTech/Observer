import { QueryClient } from '@tanstack/react-query';
import { cleanup, fireEvent, within } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Me } from '../api/generated/Me';
import type { StoredLaunch } from '../api/generated/StoredLaunch';
import { DEFAULT_LAUNCH, launchQuery } from '../api/launch';
import { en } from '../i18n/en';
import { search } from '../settings/catalog';
import { openSettings, OWNER, settle } from '../settings/fixtures';

const words = en.settings;

/** Views the dashboard and manages users, and nothing else. */
const MANAGER: Me = {
    id: 2,
    username: 'kenji',
    isOwner: false,
    permissions: '81',
    mustChangePassword: false,
};

const searchField = () =>
    document.querySelector<HTMLInputElement>('input[type="search"]') as HTMLInputElement;

/** The launch settings already read, as stored. */
const withLaunch = () => {
    const client = new QueryClient({
        defaultOptions: { queries: { staleTime: Infinity, retry: false }, mutations: { retry: false } },
    });
    const stored: StoredLaunch = {
        settings: { ...DEFAULT_LAUNCH, dataPath: 'C:\\FXServer\\server-data' },
        version: 1,
        legacyBuildPath: null,
    };
    client.setQueryData(launchQuery.queryKey, stored);
    return client;
};

describe('SettingsPage', () => {
    beforeEach(() => {
        vi.stubGlobal(
            'fetch',
            vi.fn(async () => Response.json({ code: 'not_found', params: {} }, { status: 404 })),
        );
        // jsdom lays nothing out, so it has no scrolling to the row a search leads to.
        Element.prototype.scrollIntoView = vi.fn();
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should list the categories the account may open, under their groups, the open one current', async () => {
        const { getByRole } = await openSettings('/settings/users', { me: MANAGER });
        const nav = within(getByRole('navigation', { name: words.categoriesLabel }));

        expect(nav.getAllByRole('link').map((link) => link.textContent)).toEqual([
            words.categories.restarts.name,
            words.categories.users.name,
            words.categories.account.name,
        ]);
        expect(nav.getByText(words.groups.server)).not.toBeNull();
        expect(nav.queryByText(words.groups.observer)).toBeNull();
        expect(
            nav.getByRole('link', { name: words.categories.users.name }).getAttribute('aria-current'),
        ).toBe('page');
    });

    it('should say why a category is closed to the account rather than show its settings', async () => {
        const { getByRole, getByText, queryByLabelText } = await openSettings('/settings/launch', {
            me: MANAGER,
        });

        expect(getByRole('heading', { level: 2, name: words.categories.launch.name })).not.toBeNull();
        expect(getByText(words.ownerOnly)).not.toBeNull();
        expect(queryByLabelText(words.items.dataPath.label)).toBeNull();
    });

    it('should search from /, show what matches in place of the category, and keep what was typed there', async () => {
        const { getByRole, getByLabelText, queryByRole } = await openSettings('/settings/launch', {
            client: withLaunch(),
        });
        const cfg = getByLabelText(words.items.cfgFile.label) as HTMLInputElement;
        fireEvent.change(cfg, { target: { value: 'configs/live.cfg' } });
        (document.activeElement as HTMLElement | null)?.blur();

        fireEvent.keyDown(document.body, { key: '/' });
        expect(document.activeElement).toBe(searchField());
        fireEvent.change(searchField(), { target: { value: 'password' } });

        const found = search(en, OWNER, 'password').length;
        expect(
            getByRole('heading', { level: 2, name: words.search.found(found, 'password') }),
        ).not.toBeNull();
        expect(cfg.closest('[hidden]')).not.toBeNull();
        expect(getByRole('link', { name: new RegExp(words.items.password.label) }).getAttribute('href')).toBe(
            '/settings/account#setting-password',
        );

        fireEvent.keyDown(searchField(), { key: 'Escape' });
        expect(searchField().value).toBe('');
        expect(queryByRole('heading', { name: words.search.found(found, 'password') })).toBeNull();
        expect(cfg.closest('[hidden]')).toBeNull();
        expect(cfg.value).toBe('configs/live.cfg');
    });

    it('should leave / to a field it is typed in', async () => {
        const { getByLabelText } = await openSettings('/settings/launch', { client: withLaunch() });
        const cfg = getByLabelText(words.items.cfgFile.label);

        cfg.focus();
        fireEvent.keyDown(cfg, { key: '/' });

        expect(document.activeElement).toBe(cfg);
    });

    it('should say when nothing matches', async () => {
        const { getByRole } = await openSettings('/settings/account', { me: MANAGER });

        fireEvent.change(searchField(), { target: { value: 'onesync' } });

        expect(getByRole('heading', { level: 2, name: words.search.none('onesync') })).not.toBeNull();
    });

    it('should take a result to its row, marked and brought into view, and end the search', async () => {
        const { getByRole, router } = await openSettings('/settings/account', {
            me: OWNER,
            client: withLaunch(),
        });

        fireEvent.change(searchField(), { target: { value: 'onesync' } });
        fireEvent.click(getByRole('link', { name: new RegExp(`^${words.items.onesync.label}`) }));
        await settle();
        await settle();

        expect(router.state.location.pathname).toBe('/settings/launch');
        expect(router.state.location.hash).toBe('setting-onesync');
        expect(searchField().value).toBe('');
        const row = document.getElementById('setting-onesync');
        expect(row?.className).toContain('bg-accent-soft');
        expect(row?.scrollIntoView).toHaveBeenCalledWith({ block: 'center' });
    });
});
