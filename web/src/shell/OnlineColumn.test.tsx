import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { Me } from '../api/generated/Me';
import type { OnlinePlayer } from '../api/generated/OnlinePlayer';
import { onlineQuery } from '../api/players';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { COLUMN_KEY, COLUMN_WIDE, OnlineColumn } from './OnlineColumn';

// The rows open profiles; no router runs here.
vi.mock('@tanstack/react-router', () => ({
    Link: ({
        to,
        params,
        children,
        className,
        'aria-label': label,
    }: {
        to: string;
        params?: { id: string };
        children: ReactNode;
        className?: string;
        'aria-label'?: string;
    }) => (
        <a href={to.replace('$id', params?.id ?? '')} className={className} aria-label={label}>
            {children}
        </a>
    ),
}));

const MINUTE = 60_000;
const MODERATOR: Me = {
    id: 2,
    username: 'grace',
    isOwner: false,
    permissions: '4000',
    mustChangePassword: false,
};
const ago = (ms: number) => new Date(Date.now() - ms).toISOString();

const player = (fields: Partial<OnlinePlayer> & Pick<OnlinePlayer, 'netId'>): OnlinePlayer => ({
    confirmation: 'confirmed',
    player: null,
    name: null,
    license: null,
    since: ago(MINUTE),
    sessionMs: MINUTE,
    totalMs: null,
    ping: null,
    ...fields,
});

const PLAYERS: OnlinePlayer[] = [
    player({
        netId: 3,
        player: 7,
        name: 'Ruby Vex',
        license: 'license:0123456789abcdef0123456789abcdef01234567',
        since: ago(134 * MINUTE),
        sessionMs: 134 * MINUTE,
        ping: 41,
    }),
    player({
        netId: 12,
        player: 9,
        name: 'Bastos',
        license: 'license:ffffffffffffffffffffffffffffffffffffffff',
        since: ago(47 * MINUTE),
        sessionMs: 47 * MINUTE,
        ping: 210,
    }),
    player({ netId: 5, confirmation: 'confirming', since: ago(12_000), sessionMs: 12_000 }),
];

/** The column for a moderator who sees players, on a screen wide enough for it or not. */
const column = ({ wide = true, onOpen = () => {} }: { wide?: boolean; onOpen?: () => void } = {}) => {
    vi.stubGlobal('matchMedia', (query: string) => ({
        matches: wide && query === COLUMN_WIDE,
        addEventListener: () => {},
        removeEventListener: () => {},
    }));
    const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity, retry: false } } });
    client.setQueryData(onlineQuery.queryKey, { players: PLAYERS });
    return render(
        <I18nProvider>
            <QueryClientProvider client={client}>
                <MeContext value={MODERATOR}>
                    <OnlineColumn open={false} onOpen={onOpen} onClose={() => {}} />
                </MeContext>
            </QueryClientProvider>
        </I18nProvider>,
    );
};

/** The rows as their names read, in order. */
const names = (list: HTMLElement) =>
    [...list.querySelectorAll('li')].map((row) => row.querySelector('span:nth-child(2)')?.textContent);

describe('the players column', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        localStorage.clear();
    });

    it('should list who plays by ID, name, session time and ping, each opening their profile', () => {
        const { getByRole } = column();

        const aside = within(getByRole('complementary', { name: en.online.label }));
        const ruby = aside.getByRole('link', { name: en.online.row('Ruby Vex', 3, '2h 14m', '41 ms') });

        expect(aside.getByRole('heading', { name: en.online.title })).not.toBeNull();
        expect(ruby.getAttribute('href')).toBe('/players/7');
        expect(ruby.textContent).toBe('3Ruby Vex2h 14m41 ms');
    });

    it('should mark a high ping with a word and an icon, not by colour alone', () => {
        const { getByRole } = column();

        const bastos = getByRole('link', {
            name: en.online.row('Bastos', 12, '47 min', `210 ms, ${en.online.highPing}`),
        });

        expect(within(bastos).getByText(en.online.highPing)).not.toBeNull();
        expect(bastos.querySelector('svg')).not.toBeNull();
    });

    it('should find a player by name, by whole server ID, or by licence', () => {
        const { getByRole, getByText } = column();
        const search = getByRole('searchbox', { name: en.online.search });
        const list = () => getByRole('complementary', { name: en.online.label });

        fireEvent.change(search, { target: { value: 'rUBy' } });
        const byName = names(list());
        fireEvent.change(search, { target: { value: '12' } });
        const byId = names(list());
        fireEvent.change(search, { target: { value: 'license:ffff' } });
        const byLicence = names(list());
        fireEvent.change(search, { target: { value: 'nobody' } });

        expect(byName).toEqual(['Ruby Vex']);
        expect(byId).toEqual(['Bastos']);
        expect(byLicence).toEqual(['Bastos']);
        expect(getByText(en.online.noMatch)).not.toBeNull();
    });

    it('should sort by name, ID, the longest session or the highest ping', () => {
        const { getByRole } = column();
        const list = () => getByRole('complementary', { name: en.online.label });
        const sortBy = (sort: string) => fireEvent.click(getByRole('button', { name: sort }));

        const byName = names(list());
        sortBy(en.online.sorts.id);
        const byId = names(list());
        sortBy(en.online.sorts.time);
        const byTime = names(list());
        sortBy(en.online.sorts.ping);

        expect(byName).toEqual(['Bastos', 'Ruby Vex', en.players.now.confirming]);
        expect(byId).toEqual(['Ruby Vex', en.players.now.confirming, 'Bastos']);
        expect(byTime).toEqual(['Ruby Vex', 'Bastos', en.players.now.confirming]);
        expect(names(list())).toEqual(['Bastos', 'Ruby Vex', en.players.now.confirming]);
    });

    it('should say who joined last, and how long ago', () => {
        const { getByText } = column();

        expect(getByText(en.online.joined(en.online.someone(5), '12 s'))).not.toBeNull();
    });

    it('should fold to a strip with the count, and stay folded in this browser', () => {
        const first = column();
        fireEvent.click(first.getByRole('button', { name: en.online.hide }));
        const folded = first.queryByRole('searchbox');
        const kept = localStorage.getItem(COLUMN_KEY);
        cleanup();

        const again = column();
        const strip = again.getByRole('button', { name: en.online.show('3') });
        fireEvent.click(strip);

        expect(folded).toBeNull();
        expect(kept).toBe('folded');
        expect(again.getByRole('searchbox', { name: en.online.search })).not.toBeNull();
        expect(localStorage.getItem(COLUMN_KEY)).toBeNull();
    });

    it('should open the drawer from the strip where the screen is too narrow for the column', () => {
        const onOpen = vi.fn();
        const { getByRole, queryByRole } = column({ wide: false, onOpen });

        const strip = getByRole('button', { name: en.online.show('3') });
        fireEvent.click(strip);

        expect(strip.getAttribute('aria-expanded')).toBe('false');
        expect(queryByRole('searchbox')).toBeNull();
        expect(onOpen).toHaveBeenCalledOnce();
    });
});
