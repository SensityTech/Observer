import { cleanup, fireEvent, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { MaintenancePass } from '../api/generated/MaintenancePass';
import type { MaintenancePasses } from '../api/generated/MaintenancePasses';
import type { MaintenanceSettings } from '../api/generated/MaintenanceSettings';
import type { Me } from '../api/generated/Me';
import { en } from '../i18n/en';
import { openSettings, settle } from './fixtures';

const PATH = '/api/servers/default/maintenance';
const PLAYERS = '/api/servers/default/players';
const items = en.settings.items;
const words = en.settings.maintenance;
const bar = en.settings.bar;

const STORED: MaintenanceSettings = { message: 'Back at 8, {name}.', serverName: null };
const LENA: MaintenancePass = {
    id: 2,
    license: `license:${'c'.repeat(40)}`,
    player: 12,
    name: 'Lena',
    note: null,
    by: 'maxime',
    addedAt: '2026-11-09T19:02:00Z',
};
const TESTER: MaintenancePass = {
    id: 1,
    license: `license:${'d'.repeat(40)}`,
    player: null,
    name: null,
    note: 'Tester from the dev team',
    by: 'sarah',
    addedAt: '2026-11-10T08:00:00Z',
};
const LIST: MaintenancePasses = { passes: [LENA, TESTER], zone: 'Europe/Paris' };
const NYX = { id: 14, name: 'Nyx', license: `license:${'e'.repeat(40)}`, online: false };
const NYA = { id: 15, name: 'Nya', license: `license:${'f'.repeat(40)}`, online: true };

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
const CHANGER_SEEING_PLAYERS = me('4600');

const json = (body: unknown, status = 200) => Response.json(body, { status });

const api = () =>
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = new URL(String(input), 'http://localhost');
        const method = init?.method ?? 'GET';
        if (url.pathname === PLAYERS) {
            return json({ players: [NYX, NYA], next: null, zone: 'Europe/Paris' });
        }
        if (url.pathname === `${PATH}/passes`) {
            return method === 'POST' ? json({ ...TESTER, id: 3 }, 201) : json(LIST);
        }
        if (method === 'DELETE') {
            return new Response(null, { status: 204 });
        }
        return method === 'PUT' ? json(JSON.parse(String(init?.body))) : json(STORED);
    });

const open = async (who: Me) => {
    const fetch = api();
    vi.stubGlobal('fetch', fetch);
    const view = await openSettings('/settings/maintenance', { me: who });
    // The list is asked for once the settings have arrived.
    await settle();
    await settle();
    return { ...view, fetch };
};

const sent = (fetch: ReturnType<typeof api>, method: string) =>
    fetch.mock.calls
        .filter(([, init]) => init?.method === method)
        .map(([input, init]) => ({
            path: new URL(String(input), 'http://localhost').pathname,
            body: init?.body,
        }));

const list = () => within(screen.getByRole('list', { name: items.maintenancePasses.label }));

/** Lets the pause after typing pass, and the players it asks for arrive. */
const typed = async () => {
    await new Promise((resolve) => setTimeout(resolve, 260));
    await settle();
    await settle();
};

describe('Maintenance settings', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should show the message and who still gets in to an account that cannot change them', async () => {
        const { getByText, queryByText } = await open(READER);

        const [lena, tester] = list().getAllByRole('listitem');

        expect(getByText(words.readOnly)).not.toBeNull();
        expect(getByText(words.preview).parentElement?.querySelector('p')?.textContent).toBe(
            `Back at 8, ${words.example.name}.`,
        );
        expect(within(lena!).getByText('Lena').closest('a')).toBeNull();
        expect(within(lena!).getByText(words.passes.added('maxime', '2026-11-09'))).not.toBeNull();
        expect(within(tester!).getByText('Tester from the dev team')).not.toBeNull();
        expect(screen.queryByRole('button', { name: words.passes.removeOf('Lena') })).toBeNull();
        expect(screen.queryByRole('combobox')).toBeNull();
        expect(queryByText(bar.save)).toBeNull();
    });

    it('should save the message alone, and refuse an empty one', async () => {
        const { fetch, getByText } = await open(CHANGER);
        const message = screen.getByRole('textbox', { name: items.maintenanceMessage.label });

        fireEvent.change(message, { target: { value: '  ' } });
        fireEvent.click(getByText(bar.save));
        await settle();
        const refused = screen.queryByText(words.messageMissing);
        fireEvent.change(message, { target: { value: 'Back tonight.' } });
        fireEvent.click(getByText(bar.save));
        await settle();
        await settle();

        expect(refused).not.toBeNull();
        expect(sent(fetch, 'PUT')).toEqual([
            {
                path: `${PATH}/settings`,
                body: JSON.stringify({ message: 'Back tonight.', serverName: null }),
            },
        ]);
        expect(getByText(words.saved)).not.toBeNull();
    });

    it('should give the server a name for the list, no longer than the list keeps', async () => {
        const { fetch, getByText } = await open(CHANGER);
        const name = screen.getByRole('textbox', { name: items.maintenanceServerName.label });

        fireEvent.change(name, { target: { value: 'n'.repeat(41) } });
        fireEvent.click(getByText(bar.save));
        await settle();
        const refused = screen.queryByText(words.nameTooLong(40));
        fireEvent.change(name, { target: { value: ' Aperture RP | maintenance ' } });
        fireEvent.click(getByText(bar.save));
        await settle();
        await settle();

        expect(refused).not.toBeNull();
        expect(sent(fetch, 'PUT')).toEqual([
            {
                path: `${PATH}/settings`,
                body: JSON.stringify({ message: STORED.message, serverName: 'Aperture RP | maintenance' }),
            },
        ]);
    });

    it('should take a pasted licence with a note, and nothing that is neither a licence nor a player picked', async () => {
        const { fetch } = await open(CHANGER);
        const field = screen.getByRole('combobox', { name: words.passes.fieldLicence });

        fireEvent.change(field, { target: { value: 'Nyx' } });
        fireEvent.click(screen.getByRole('button', { name: words.passes.add }));
        const refused = screen.queryByText(words.passes.pickFirst);
        fireEvent.change(field, { target: { value: ` ${'D'.repeat(40)} ` } });
        fireEvent.change(screen.getByLabelText(new RegExp(words.passes.note)), {
            target: { value: ' Tester from the dev team ' },
        });
        fireEvent.click(screen.getByRole('button', { name: words.passes.add }));
        await settle();
        await settle();

        expect(refused).not.toBeNull();
        expect(sent(fetch, 'POST')).toEqual([
            {
                path: `${PATH}/passes`,
                body: JSON.stringify({ license: 'D'.repeat(40), note: 'Tester from the dev team' }),
            },
        ]);
        expect(screen.getByText(words.passes.addedNotice('Tester from the dev team'))).not.toBeNull();
        expect((field as HTMLInputElement).value).toBe('');
    });

    it('should list the players a name finds, those online first, and add the one picked', async () => {
        const { fetch } = await open(CHANGER_SEEING_PLAYERS);
        const field = screen.getByRole('combobox', { name: words.passes.field });

        fireEvent.change(field, { target: { value: 'Ny' } });
        await typed();
        const options = screen.getAllByRole('option').map((option) => option.textContent);
        fireEvent.keyDown(field, { key: 'ArrowDown' });
        fireEvent.keyDown(field, { key: 'Enter' });
        const picked = screen.getByText(words.passes.picked('Nya', 'ffffffff…'));
        fireEvent.keyDown(field, { key: 'Enter' });
        await settle();
        await settle();

        const asked = fetch.mock.calls
            .map(([input]) => new URL(String(input), 'http://localhost'))
            .filter((url) => url.pathname === PLAYERS)
            .map((url) => url.search);
        expect(asked).toEqual(['?search=Ny&sort=lastSeen']);
        expect(options).toEqual([`Nyaffffffff…${words.passes.online}`, 'Nyxeeeeeeee…']);
        expect(picked).not.toBeNull();
        expect(sent(fetch, 'POST')).toEqual([
            { path: `${PATH}/passes`, body: JSON.stringify({ license: NYA.license, note: null }) },
        ]);
    });

    it('should take a player off the list in one press and say so', async () => {
        const { fetch } = await open(CHANGER_SEEING_PLAYERS);
        const profile = list().getByRole('link', { name: 'Lena' }).getAttribute('href');

        fireEvent.click(screen.getByRole('button', { name: words.passes.removeOf('Lena') }));
        await settle();
        await settle();

        expect(profile).toBe('/players/12');
        expect(sent(fetch, 'DELETE')).toEqual([{ path: `${PATH}/passes/2`, body: undefined }]);
        expect(screen.getByText(words.passes.removedNotice('Lena'))).not.toBeNull();
    });
});
