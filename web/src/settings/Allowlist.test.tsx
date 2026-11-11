import { cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { AllowlistSettings } from '../api/generated/AllowlistSettings';
import type { Me } from '../api/generated/Me';
import { en } from '../i18n/en';
import { openSettings, settle } from './fixtures';

const PATH = '/api/servers/default/allowlist/settings';
const items = en.settings.items;
const words = en.settings.allowlist;
const bar = en.settings.bar;

const STORED: AllowlistSettings = {
    enabled: false,
    message: 'Approved players only.\nGive the staff this ID: {id}',
};

const me = (permissions: string): Me => ({
    id: 2,
    username: 'kenji',
    isOwner: false,
    permissions,
    mustChangePassword: false,
});
/** Each with what it implies, as the API sends a set. */
const READER = me('200');
const CHANGER = me('600');

const json = (body: unknown, status = 200) => Response.json(body, { status });

const api = () =>
    vi.fn(async (_: RequestInfo | URL, init?: RequestInit) =>
        init?.method === 'PUT' ? json(JSON.parse(String(init.body))) : json(STORED),
    );

const open = async (who: Me) => {
    const fetch = api();
    vi.stubGlobal('fetch', fetch);
    const view = await openSettings('/settings/allowlist', { me: who });
    return { ...view, fetch };
};

const put = (fetch: ReturnType<typeof api>) =>
    fetch.mock.calls
        .filter(([, init]) => init?.method === 'PUT')
        .map(([input, init]) => ({
            path: new URL(String(input), 'http://localhost').pathname,
            body: init?.body,
        }));

describe('Allowlist settings', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should show the message as a refused player reads it, to an account that cannot change it', async () => {
        const { getByText, queryByText } = await open(READER);
        const enabled = screen.getByRole('switch', {
            name: items.allowlistEnabled.label,
        }) as HTMLInputElement;

        expect(enabled.checked).toBe(false);
        expect(enabled.matches(':disabled')).toBe(true);
        expect(getByText(words.readOnly)).not.toBeNull();
        expect(getByText(words.preview).parentElement?.querySelector('p')?.textContent).toBe(
            `Approved players only.\nGive the staff this ID: ${words.example.id}`,
        );
        expect(queryByText(bar.save)).toBeNull();
        expect(screen.queryByRole('link', { name: words.openAllowlist })).toBeNull();
    });

    it('should turn the allowlist on with the message, saved together', async () => {
        const { fetch, getByText } = await open(CHANGER);

        const message = screen.getByRole('textbox', { name: items.allowlistMessage.label });
        fireEvent.click(screen.getByRole('switch', { name: items.allowlistEnabled.label }));
        (message as HTMLTextAreaElement).setSelectionRange(STORED.message.length, STORED.message.length);
        fireEvent.click(screen.getByRole('button', { name: '{name}' }));
        fireEvent.click(getByText(bar.save));
        await settle();
        await settle();

        expect(put(fetch)).toEqual([
            {
                path: PATH,
                body: JSON.stringify({ enabled: true, message: `${STORED.message}{name}` }),
            },
        ]);
        expect(getByText(words.saved)).not.toBeNull();
    });

    it('should warn about a message without the request ID, and refuse an empty one', async () => {
        const { fetch, getByText } = await open(CHANGER);
        const message = screen.getByRole('textbox', { name: items.allowlistMessage.label });

        fireEvent.change(message, { target: { value: 'Closed.' } });
        const warned = screen.getByText(words.idMissing);
        fireEvent.change(message, { target: { value: '   ' } });
        fireEvent.click(getByText(bar.save));
        await settle();

        expect(warned).not.toBeNull();
        expect(getByText(words.messageMissing)).not.toBeNull();
        expect(put(fetch)).toEqual([]);
    });
});
