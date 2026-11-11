import { cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { EntitiesSettings } from '../api/generated/EntitiesSettings';
import type { Me } from '../api/generated/Me';
import { Permission } from '../api/generated/Permission';
import { en } from '../i18n/en';
import { openSettings } from './fixtures';

const words = en.settings.entities;

/** Reads the settings without the permission to change them. */
const READER: Me = {
    id: 2,
    username: 'grace',
    isOwner: false,
    permissions: Permission.SettingsView.toString(16),
    mustChangePassword: false,
};

/** The Entities category over a server that lists every `now.everySeconds`, each save kept in the list returned. */
const serve = (now: EntitiesSettings) => {
    const sent: unknown[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string, init: RequestInit) => {
            const route = `${init.method ?? 'GET'} ${String(input).replace(/^\/api/, '')}`;
            if (route === 'GET /servers/default/entities/settings') {
                return Response.json(now);
            }
            if (route === 'PUT /servers/default/entities/settings') {
                const body = JSON.parse(String(init.body)) as EntitiesSettings;
                sent.push(body);
                return Response.json(body);
            }
            return Response.json({ code: 'not_found', params: {} }, { status: 404 });
        }),
    );
    return sent;
};

const cadence = (seconds: number) =>
    screen.getByRole<HTMLInputElement>('radio', { name: words.seconds(seconds) });

describe('Entities settings', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should show how often the server lists its entities, and save another cadence', async () => {
        const sent = serve({ everySeconds: 15 });
        await openSettings('/settings/entities');

        expect(cadence(15).checked).toBe(true);
        expect(screen.queryByRole('button', { name: en.settings.bar.save })).toBeNull();

        fireEvent.click(cadence(30));
        fireEvent.click(screen.getByRole('button', { name: en.settings.bar.save }));

        await waitFor(() => expect(screen.getByText(words.saved)).toBeTruthy());
        expect(sent).toEqual([{ everySeconds: 30 }]);
        expect(cadence(30).checked).toBe(true);
        expect(screen.queryByRole('button', { name: en.settings.bar.save })).toBeNull();
    });

    it('should go back to the cadence kept when the change is discarded', async () => {
        const sent = serve({ everySeconds: 15 });
        await openSettings('/settings/entities');

        fireEvent.click(cadence(60));
        fireEvent.click(screen.getByRole('button', { name: en.settings.bar.discard }));

        expect(cadence(15).checked).toBe(true);
        expect(sent).toEqual([]);
    });

    it('should let an account without the permission to change settings read the cadence only', async () => {
        serve({ everySeconds: 10 });
        await openSettings('/settings/entities', { me: READER });

        expect(screen.getByText(words.readOnly)).toBeTruthy();
        expect(cadence(10).checked).toBe(true);
        expect(cadence(30).matches(':disabled')).toBe(true);
    });
});
