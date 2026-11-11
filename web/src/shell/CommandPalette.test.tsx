import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { Me } from '../api/generated/Me';
import type { OnlinePlayers } from '../api/generated/OnlinePlayers';
import { onlineQuery } from '../api/players';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { CommandPalette } from './CommandPalette';

const navigate = vi.hoisted(() => vi.fn());
vi.mock('@tanstack/react-router', () => ({ useNavigate: () => navigate }));

const OWNER: Me = { id: 1, username: 'ada', isOwner: true, permissions: 'fffff', mustChangePassword: false };
/** Sees the dashboard and nothing else. */
const VIEWER: Me = { ...OWNER, id: 2, username: 'grace', isOwner: false, permissions: '1' };

const ONLINE = {
    players: [
        { netId: 3, player: 41, name: 'Ruby Vex', label: null },
        { netId: 7, player: null, name: 'kaito', label: null },
    ],
} as unknown as OnlinePlayers;

function Harness({ me }: { me: Me }) {
    const [open, setOpen] = useState(false);
    return <CommandPalette me={me} open={open} onOpen={() => setOpen(true)} onClose={() => setOpen(false)} />;
}

/** The results on screen, by their name. */
const listed = () =>
    [...document.querySelectorAll('dialog li button')].map(
        (button) => button.firstElementChild!.nextSibling!.textContent,
    );

const palette = (me: Me = OWNER) => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } });
    client.setQueryData(onlineQuery.queryKey, ONLINE);
    const view = render(
        <QueryClientProvider client={client}>
            <I18nProvider>
                <Harness me={me} />
            </I18nProvider>
        </QueryClientProvider>,
    );
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
    const field = () => view.getByRole('searchbox', { name: en.palette.label });
    return { ...view, field };
};

describe('CommandPalette', () => {
    beforeAll(() => {
        // jsdom has the element but not its modal methods.
        HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
            this.open = true;
        };
        HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
            this.open = false;
        };
    });

    afterEach(() => {
        cleanup();
        navigate.mockReset();
    });

    it('should open on Ctrl K with the pages the account opens, and close on the same keys', () => {
        palette();
        expect(listed()).toContain(en.nav.cfg);
        expect(listed()).toContain(en.tools.hub.heavy.title);
        fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
        expect(document.querySelector('dialog')).toBeNull();
        cleanup();

        palette(VIEWER);
        expect(listed()).toEqual([
            en.nav.overview,
            en.nav.resources,
            en.nav.schedule,
            en.nav.statistics,
            en.nav.settings,
        ]);
    });

    it('should go to the page the arrows are on when Enter is pressed', () => {
        const { field } = palette();

        fireEvent.keyDown(field(), { key: 'ArrowDown' });
        fireEvent.keyDown(field(), { key: 'ArrowDown' });
        fireEvent.keyDown(field(), { key: 'ArrowUp' });
        fireEvent.keyDown(field(), { key: 'Enter' });

        expect(navigate).toHaveBeenCalledWith({ to: '/console' });
        expect(document.querySelector('dialog')).toBeNull();
    });

    it('should find a setting by its name and open it in its category', () => {
        const { field, getByText } = palette();

        fireEvent.change(field(), { target: { value: 'stops responding' } });

        expect(getByText(en.settings.items.hangDetection.label)).not.toBeNull();
        expect(getByText(en.palette.setting(en.settings.categories.launch.name))).not.toBeNull();
        fireEvent.keyDown(field(), { key: 'Enter' });
        expect(navigate).toHaveBeenCalledWith({
            to: '/settings/$category',
            params: { category: 'launch' },
            hash: 'setting-hangDetection',
        });
    });

    it('should find a connected player whose profile is known, by name or by ID', () => {
        const { field, getByText } = palette();

        fireEvent.change(field(), { target: { value: 'ruby' } });
        expect(listed()).toEqual(['Ruby Vex']);
        fireEvent.click(getByText('Ruby Vex'));
        expect(navigate).toHaveBeenCalledWith({ to: '/players/$id', params: { id: '41' } });
    });

    it('should say that nothing matches, and leave players out for an account that does not see them', () => {
        const { field, getByText } = palette(VIEWER);

        fireEvent.change(field(), { target: { value: 'ruby' } });

        expect(listed()).toEqual([]);
        expect(getByText(en.palette.none)).not.toBeNull();
    });
});
