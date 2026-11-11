import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { Target } from '../api/moderation';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { AnnounceDialog, KickDialog, MessageDialog } from './dialogs';

const KAITO: Target = {
    netId: 12,
    name: 'kaito',
    license: 'license:00000000000000000000000000000000000000aa',
};

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** An API that takes every action, or refuses them all with `refusal`. */
const api = (refusal?: { status: number; code: string }) =>
    vi.fn(async (_input: RequestInfo | URL, _init?: RequestInit) =>
        refusal === undefined
            ? new Response(null, { status: 204 })
            : Response.json({ code: refusal.code, params: {} }, { status: refusal.status }),
    );

const show = (dialog: ReactNode, fetch = api()) => {
    vi.stubGlobal('fetch', fetch);
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
        <I18nProvider>
            <QueryClientProvider client={client}>{dialog}</QueryClientProvider>
        </I18nProvider>,
    );
    return fetch;
};

const sent = (fetch: ReturnType<typeof api>) =>
    fetch.mock.calls.map(([input, init]) => ({
        method: init?.method,
        path: new URL(String(input), 'http://localhost').pathname,
        body: JSON.parse(String(init?.body)) as unknown,
    }));

describe('the dialogs that act on connected players', () => {
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
        vi.unstubAllGlobals();
    });

    it('should kick a player by server ID and licence, for a reason, then say they were kicked', async () => {
        const closed = vi.fn();
        const fetch = show(<KickDialog target={KAITO} onClose={closed} />);
        const dialog = within(screen.getByRole('dialog', { name: en.moderation.kickDialog.title('kaito') }));

        fireEvent.click(dialog.getByRole('button', { name: en.moderation.kickDialog.submit }));
        expect(dialog.getByText(en.moderation.kickDialog.reasonMissing)).not.toBeNull();
        expect(dialog.getByText(en.moderation.player(12, KAITO.license))).not.toBeNull();
        expect(sent(fetch)).toEqual([]);
        fireEvent.change(dialog.getByLabelText(en.moderation.kickDialog.reason), {
            target: { value: '  RDM at the hospital ' },
        });
        fireEvent.click(dialog.getByRole('button', { name: en.moderation.kickDialog.submit }));
        await settle();
        await settle();

        expect(sent(fetch)).toEqual([
            {
                method: 'POST',
                path: '/api/servers/default/kicks',
                body: { player: 12, license: KAITO.license, reason: 'RDM at the hospital' },
            },
        ]);
        expect(screen.getByText(en.moderation.kickDialog.done('kaito'))).not.toBeNull();
        expect(screen.queryByRole('button', { name: en.moderation.kickDialog.submit })).toBeNull();
        fireEvent.click(screen.getByRole('button', { name: en.common.close }));
        expect(closed).toHaveBeenCalledOnce();
    });

    it('should keep the dialog and say why when the player left before the kick', async () => {
        const fetch = show(
            <KickDialog target={KAITO} onClose={() => {}} />,
            api({ status: 409, code: 'player_not_connected' }),
        );

        fireEvent.change(screen.getByLabelText(en.moderation.kickDialog.reason), {
            target: { value: 'AFK' },
        });
        fireEvent.click(screen.getByRole('button', { name: en.moderation.kickDialog.submit }));
        await settle();
        await settle();

        expect(fetch).toHaveBeenCalledOnce();
        expect(screen.getByRole('alert').textContent).toBe(en.errors.player_not_connected);
        expect(screen.getByRole('button', { name: en.moderation.kickDialog.submit })).not.toBeNull();
    });

    it('should refuse a reason longer than the server takes, counting characters', () => {
        const fetch = show(<KickDialog target={KAITO} onClose={() => {}} />);

        fireEvent.change(screen.getByLabelText(en.moderation.kickDialog.reason), {
            target: { value: '🚗'.repeat(201) },
        });
        fireEvent.click(screen.getByRole('button', { name: en.moderation.kickDialog.submit }));

        expect(screen.getByText(en.moderation.counter(201, 200))).not.toBeNull();
        expect(screen.getByText(en.moderation.kickDialog.reasonMissing)).not.toBeNull();
        expect(fetch).not.toHaveBeenCalled();
    });

    it('should send an announcement and say that the resources of the server show it', async () => {
        const fetch = show(<AnnounceDialog onClose={() => {}} />);

        expect(screen.getByText(en.moderation.raised('observer:announcement'))).not.toBeNull();
        fireEvent.change(screen.getByLabelText(en.moderation.announce.text), {
            target: { value: 'Restart in five minutes\nPark your cars' },
        });
        fireEvent.click(screen.getByRole('button', { name: en.moderation.announce.submit }));
        await settle();
        await settle();

        expect(sent(fetch)).toEqual([
            {
                method: 'POST',
                path: '/api/servers/default/announcements',
                body: { message: 'Restart in five minutes\nPark your cars' },
            },
        ]);
        expect(screen.getByText(en.moderation.announce.done)).not.toBeNull();
    });

    it('should send a message to one player, addressed like a kick', async () => {
        const fetch = show(<MessageDialog target={{ ...KAITO, license: null }} onClose={() => {}} />);

        expect(screen.getByText(en.moderation.raised('observer:directMessage'))).not.toBeNull();
        expect(screen.getByText(en.moderation.player(12, null))).not.toBeNull();
        fireEvent.change(screen.getByLabelText(en.moderation.messageDialog.text), {
            target: { value: 'Come to the staff room' },
        });
        fireEvent.click(screen.getByRole('button', { name: en.moderation.messageDialog.submit }));
        await settle();
        await settle();

        expect(sent(fetch)).toEqual([
            {
                method: 'POST',
                path: '/api/servers/default/messages',
                body: { player: 12, license: null, message: 'Come to the staff room' },
            },
        ]);
        expect(screen.getByText(en.moderation.messageDialog.done('kaito'))).not.toBeNull();
    });
});
