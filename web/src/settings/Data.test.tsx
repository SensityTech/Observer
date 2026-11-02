import { act, cleanup, fireEvent, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { ObserverSettings } from '../api/generated/ObserverSettings';
import type { PlayersImport } from '../api/generated/PlayersImport';
import { importQuery } from '../api/players';
import { SETTINGS as CHANGES } from '../changes/fixtures';
import { SETTINGS as DROPS } from '../drops/fixtures';
import { en } from '../i18n/en';
import { SETTINGS as PLAYERS } from '../players/fixtures';
import { openSettings, settle } from './fixtures';

const items = en.settings.items;
const bar = en.settings.bar;

const OBSERVER: ObserverSettings = {
    configFile: 'C:\\Observer\\observer.toml',
    listen: '0.0.0.0:40125',
    hosts: [],
    trustedProxies: [],
    statistics: { samples: 72 * 3600, minutes: 30 * 86_400 },
};

const writing: PlayersImport = {
    state: 'writing',
    startedAt: '2026-10-30T20:00:00Z',
    finishedAt: null,
    bytes: 3_145_728,
    read: 5_000,
    written: 1_200,
    imported: 0,
    updated: 0,
    forgotten: 0,
    invalid: 0,
};

/** Observer's answers for the Data kept category; every request made is kept, with its body. */
const serve = (players: () => PlayersImport | null = () => null) => {
    const asked: { route: string; body: unknown }[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string, init: RequestInit) => {
            const route = `${init.method ?? 'GET'} ${String(input).replace(/^\/api/, '')}`;
            const body = typeof init.body === 'string' ? JSON.parse(init.body) : undefined;
            asked.push({ route, body });
            switch (route) {
                case 'GET /servers/default/changes/settings':
                    return Response.json(CHANGES);
                case 'PUT /servers/default/changes/settings':
                    return Response.json({ ...CHANGES, settings: body });
                case 'GET /servers/default/drops/settings':
                    return Response.json(DROPS);
                case 'PUT /servers/default/drops/settings':
                    return Response.json(body);
                case 'GET /servers/default/players/settings':
                    return Response.json(PLAYERS);
                case 'GET /servers/default/players/import':
                    return Response.json(players());
                case 'GET /settings/observer':
                    return Response.json(OBSERVER);
                default:
                    return Response.json({ code: 'not_found', params: {} }, { status: 404 });
            }
        }),
    );
    return asked;
};

/** The card of one section of the category, by its title. */
const group = (title: string) => {
    const heading = [...document.querySelectorAll('h3')].find((each) => each.textContent === title);
    const section = heading?.closest('section');
    if (section == null) {
        throw new Error(`no group ${title}`);
    }
    return within(section);
};

describe('Data kept settings', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should show what the change tracking keeps, with the patterns that match no file', async () => {
        serve();
        await openSettings('/settings/data');

        const changes = group(en.settings.data.changes);
        expect(changes.getByText('*.secret', { selector: 'code' }).nextSibling?.textContent).toBe(
            en.changes.settings.matchesNone,
        );
        expect(changes.getByText('cache', { selector: 'code' }).nextSibling?.textContent).toBe(
            en.changes.settings.matchesNone,
        );
        expect(changes.getByText(en.changes.settings.matches(1))).not.toBeNull();
        expect((changes.getByLabelText(items.budget.label) as HTMLInputElement).value).toBe('2');
    });

    it('should count the changes of every section in one bar, and save each with its own request', async () => {
        const asked = serve();
        const { getByRole, getByText } = await openSettings('/settings/data');

        fireEvent.change(group(en.settings.data.changes).getByLabelText(items.maxText.label), {
            target: { value: '2' },
        });
        fireEvent.change(group(en.settings.data.drops).getByLabelText(items.sessionDays.label), {
            target: { value: '120' },
        });
        getByText(bar.changes(2));
        getByText(
            `· ${bar.applies([en.settings.appliesWhen.withinHour, en.settings.appliesWhen.nextStart])}`,
        );
        fireEvent.click(getByRole('button', { name: bar.save }));

        await waitFor(() => getByText(en.drops.settings.saved));
        getByText(en.changes.settings.saved);
        const puts = asked.filter(({ route }) => route.startsWith('PUT'));
        expect(puts.map(({ route }) => route).toSorted()).toEqual([
            'PUT /servers/default/changes/settings',
            'PUT /servers/default/drops/settings',
        ]);
        expect(puts.find(({ route }) => route.includes('drops'))?.body).toEqual({ sessionDays: 120 });
    });

    it('should drop the changes of every section on discard, and the bar with them', async () => {
        serve();
        const { getByRole, queryByText } = await openSettings('/settings/data');
        const maxText = group(en.settings.data.changes).getByLabelText(
            items.maxText.label,
        ) as HTMLInputElement;
        const sessions = group(en.settings.data.drops).getByLabelText(
            items.sessionDays.label,
        ) as HTMLInputElement;

        fireEvent.change(maxText, { target: { value: '2' } });
        fireEvent.change(sessions, { target: { value: '120' } });
        fireEvent.click(getByRole('button', { name: bar.discard }));

        expect(maxText.value).toBe('1');
        expect(sessions.value).toBe(String(DROPS.sessionDays));
        expect(queryByText(bar.changes(2))).toBeNull();
        expect(queryByText(bar.save)).toBeNull();
    });

    it('should keep an emptied retention empty, and refuse it as out of range without sending it', async () => {
        const asked = serve();
        const { getByRole } = await openSettings('/settings/data');
        const players = group(en.settings.data.players);
        const links = players.getByLabelText(items.linkDays.label);
        expect(links.getAttribute('min')).toBe('30');

        fireEvent.change(links, { target: { value: '' } });
        expect((links as HTMLInputElement).value).toBe('');
        fireEvent.click(getByRole('button', { name: bar.save }));
        await settle();

        expect(players.getByText(en.players.data.outOfRange(en.players.data.ranges.linkDays))).not.toBeNull();
        expect(asked.some(({ route }) => route.startsWith('PUT'))).toBe(false);
    });

    it('should show the import while it writes, then what it took in', async () => {
        let current = writing;
        serve(() => current);
        const { client } = await openSettings('/settings/data');

        const players = group(en.settings.data.players);
        expect(players.getByText(en.players.data.writing(1_200, 5_000))).not.toBeNull();
        expect(
            players.getByRole('progressbar', { name: en.players.data.progress }).getAttribute('value'),
        ).toBe('1200');
        expect(players.getByRole('button', { name: en.players.data.start }).hasAttribute('disabled')).toBe(
            true,
        );

        current = {
            ...writing,
            state: 'finished',
            finishedAt: '2026-10-30T20:01:00Z',
            written: 5_000,
            imported: 4_700,
            updated: 280,
            forgotten: 12,
            invalid: 8,
        };
        await act(() => client.refetchQueries({ queryKey: importQuery.queryKey }));
        await settle();

        expect(players.getByText(/^Imported .*: 4,700 new profiles, 280 updated, 12 skipped/)).not.toBeNull();
        expect(players.queryByRole('progressbar')).toBeNull();
    });

    it('should start an import with Enter in its field rather than save the settings', async () => {
        const asked = serve();
        await openSettings('/settings/data');
        const path = group(en.settings.data.players).getByLabelText(en.players.data.path);

        fireEvent.keyDown(path, { key: 'Enter' });
        expect(group(en.settings.data.players).getByText(en.validation.required)).not.toBeNull();

        fireEvent.change(path, { target: { value: 'C:\\txData\\default\\data\\playersDB.json' } });
        fireEvent.keyDown(path, { key: 'Enter' });
        await settle();
        expect(asked.find(({ route }) => route === 'POST /servers/default/players/import')?.body).toEqual({
            path: 'C:\\txData\\default\\data\\playersDB.json',
        });
    });

    it('should say how long the statistics stay, which nothing here changes', async () => {
        serve();
        await openSettings('/settings/data');

        const statistics = group(en.settings.data.statistics);
        expect(statistics.getByText(en.settings.data.samples(3))).not.toBeNull();
        expect(statistics.getByText(en.settings.data.minutes(30))).not.toBeNull();
        expect(statistics.getByText(en.settings.data.hours)).not.toBeNull();
        expect(statistics.queryByRole('textbox')).toBeNull();
    });
});
