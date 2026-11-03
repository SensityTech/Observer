import { act, cleanup, fireEvent, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import { ArtifactsProvider } from '../artifacts/context';
import { ARTIFACTS } from '../artifacts/fixtures';
import { en } from '../i18n/en';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { openSettings, settle } from './fixtures';

const items = en.settings.items;
const bar = en.settings.bar;

const STATUS = { ...(structuredClone(ARTIFACTS) as unknown as ArtifactsStatus), job: null };

/** The FXServer builds category for an account holding `permissions`, the builds as the topic says. */
const page = async (permissions: string) => {
    let socket: SocketEvents | undefined;
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: () => {}, close: () => {} };
        },
        refetch: () => {},
    });
    const view = await openSettings('/settings/builds', {
        me: { id: 2, username: 'kenji', isOwner: false, permissions, mustChangePassword: false },
        around: (content: ReactNode) => (
            <LiveContext value={live}>
                <ArtifactsProvider>{content}</ArtifactsProvider>
            </LiveContext>
        ),
    });
    live.start();
    await settle();
    const say = (message: unknown) => act(() => socket?.message(JSON.stringify(message)));
    say({ type: 'hello', connectionId: 1, contract: 1 });
    say({ type: 'snapshot', topic: 'server.artifacts', data: STATUS });
    return { ...view, live };
};

describe('FXServer builds settings', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should keep a count of builds whole before sending the settings', async () => {
        const fetch = vi.fn(async () => Response.json(STATUS));
        vi.stubGlobal('fetch', fetch);
        const { getByLabelText, getByRole, getByText, live } = await page('60');

        fireEvent.change(getByLabelText(items.keepBuilds.label), { target: { value: '2.5' } });
        fireEvent.click(getByRole('button', { name: bar.save }));

        expect(getByText(en.artifacts.settings.problems.outOfRange)).toBeTruthy();
        expect(fetch).not.toHaveBeenCalled();
        live.stop();
    });

    it('should say the count of builds is only for a cleanup, and save the settings over their version', async () => {
        const fetch = vi.fn(async () =>
            Response.json({ settings: { ...STATUS.settings, keepBuilds: 5 }, version: 2 }),
        );
        vi.stubGlobal('fetch', fetch);
        const { getByLabelText, getByRole, getByText, live } = await page('60');

        fireEvent.change(getByLabelText(items.keepBuilds.label), { target: { value: '5' } });
        getByText(`· ${bar.applies([en.settings.appliesWhen.cleanup])}`);
        fireEvent.click(getByRole('button', { name: bar.save }));

        await waitFor(() => getByText(en.artifacts.settings.saved));
        const [path, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
        expect(path).toBe('/api/servers/default/artifacts/settings');
        expect(JSON.parse(String(init.body))).toEqual({
            settings: { ...STATUS.settings, keepBuilds: 5 },
            version: STATUS.version,
        });
        live.stop();
    });

    it('should show the settings to an account that sees builds, without letting it change them', async () => {
        const { getByLabelText, getByText, queryByText, live } = await page('20');

        expect(getByText(en.artifacts.settings.readOnly)).toBeTruthy();
        expect(getByLabelText(items.keepBuilds.label).matches(':disabled')).toBe(true);
        expect(queryByText(bar.save)).toBeNull();
        live.stop();
    });
});
