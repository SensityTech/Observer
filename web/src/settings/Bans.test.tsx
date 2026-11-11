import { cleanup, fireEvent, screen, within } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { BansSettings } from '../api/generated/BansSettings';
import type { Me } from '../api/generated/Me';
import { en } from '../i18n/en';
import { openSettings, settle } from './fixtures';

const PATH = '/api/servers/default/bans/settings';
const words = en.settings.bans;
const bar = en.settings.bar;
const lengths = en.bans.dialog.lengths;

const STORED: BansSettings = {
    message: 'Banned: {reason}',
    tokenMatches: 1,
    templates: [
        { reason: 'Cheating', minutes: null },
        { reason: 'Insults in voice chat', minutes: 3 * 24 * 60 },
    ],
};

/** SETTINGS_VIEW alone. */
const VIEWER: Me = {
    id: 2,
    username: 'kenji',
    isOwner: false,
    permissions: '200',
    mustChangePassword: false,
};

const serve = (stored: BansSettings = STORED) => {
    const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        if (String(input) !== PATH) {
            return Response.json({ code: 'not_found', params: {} }, { status: 404 });
        }
        return Response.json(init?.method === 'PUT' ? JSON.parse(String(init.body)) : stored);
    });
    vi.stubGlobal('fetch', fetch);
    return fetch;
};

const saved = (fetch: ReturnType<typeof serve>) =>
    fetch.mock.calls
        .filter(([, init]) => init?.method === 'PUT')
        .map(([, init]) => JSON.parse(String(init?.body)) as BansSettings);

/** Each template as the list reads: its reason, then its length. */
const listed = () =>
    within(screen.getByRole('list', { name: words.templates }))
        .getAllByRole('listitem')
        .map((item) => [...item.children].slice(0, 2).map((part) => part.textContent));

const press = (name: string) => fireEvent.click(screen.getByRole('button', { name }));

/** A template as its buttons name it. */
const named = en.bans.dialog.templateOption;

describe('Bans settings', () => {
    beforeAll(() => {
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

    it('should list the templates in the order the ban dialog offers them, each with its length', async () => {
        serve();
        await openSettings('/settings/bans');

        expect(listed()).toEqual([
            ['Cheating', 'Permanent'],
            ['Insults in voice chat', '3 days'],
        ]);
        expect(screen.queryByRole('button', { name: bar.save })).toBeNull();
    });

    it('should add a template, move it up and save the list as it is ordered', async () => {
        const fetch = serve();
        await openSettings('/settings/bans');

        press(words.addTemplate);
        const dialog = within(screen.getByRole('dialog'));
        fireEvent.click(dialog.getByRole('button', { name: words.templateDialog.submitAdd }));
        expect(dialog.getByText(en.bans.dialog.reasonMissing)).not.toBeNull();
        fireEvent.change(dialog.getByLabelText(en.bans.dialog.reason), {
            target: { value: '  Combat logging ' },
        });
        fireEvent.click(dialog.getByRole('button', { name: lengths.week }));
        fireEvent.click(dialog.getByRole('button', { name: words.templateDialog.submitAdd }));
        await settle();

        expect(screen.queryByRole('dialog')).toBeNull();
        expect(saved(fetch)).toEqual([]);
        expect(listed().at(-1)).toEqual(['Combat logging', '7 days']);
        press(words.moveUp(named('Combat logging', '7 days')));
        press(words.moveUp(named('Combat logging', '7 days')));
        expect(
            screen.getByRole('button', { name: words.moveUp(named('Combat logging', '7 days')) }),
        ).toHaveProperty('disabled', true);
        press(bar.save);
        await settle();
        await settle();

        expect(saved(fetch)).toEqual([
            {
                ...STORED,
                templates: [{ reason: 'Combat logging', minutes: 7 * 24 * 60 }, ...STORED.templates],
            },
        ]);
        expect(screen.getByText(words.saved)).not.toBeNull();
        expect(screen.queryByRole('button', { name: bar.save })).toBeNull();
    });

    it('should edit a template, remove another and count both as one change', async () => {
        const fetch = serve();
        await openSettings('/settings/bans');

        press(words.editTemplate(named('Cheating', 'Permanent')));
        const dialog = within(screen.getByRole('dialog'));
        expect(dialog.getByLabelText<HTMLTextAreaElement>(en.bans.dialog.reason).value).toBe('Cheating');
        expect(dialog.getByRole('button', { name: lengths.permanent }).getAttribute('aria-pressed')).toBe(
            'true',
        );
        fireEvent.click(dialog.getByRole('button', { name: lengths.other }));
        fireEvent.change(dialog.getByLabelText(en.bans.dialog.amount), { target: { value: '45' } });
        fireEvent.change(dialog.getByLabelText(en.bans.dialog.unit), { target: { value: 'days' } });
        fireEvent.click(dialog.getByRole('button', { name: words.templateDialog.submitEdit }));
        await settle();
        press(words.removeTemplate(named('Insults in voice chat', '3 days')));

        expect(listed()).toEqual([['Cheating', '45 days']]);
        expect(screen.getByText(bar.changes(1))).not.toBeNull();
        press(bar.save);
        await settle();
        await settle();
        expect(saved(fetch)[0]?.templates).toEqual([{ reason: 'Cheating', minutes: 45 * 24 * 60 }]);
    });

    it('should refuse a template listed already, and put the list back on a discard', async () => {
        serve();
        await openSettings('/settings/bans');

        press(words.addTemplate);
        const dialog = within(screen.getByRole('dialog'));
        fireEvent.change(dialog.getByLabelText(en.bans.dialog.reason), { target: { value: 'Cheating' } });
        fireEvent.click(dialog.getByRole('button', { name: lengths.permanent }));
        fireEvent.click(dialog.getByRole('button', { name: words.templateDialog.submitAdd }));
        expect(dialog.getByText(words.templateDialog.listed)).not.toBeNull();
        fireEvent.click(dialog.getByRole('button', { name: lengths.hour }));
        fireEvent.click(dialog.getByRole('button', { name: words.templateDialog.submitAdd }));
        await settle();
        expect(listed()).toHaveLength(3);
        press(words.moveDown(named('Cheating', 'Permanent')));
        press(bar.discard);
        await settle();

        expect(listed()).toEqual([
            ['Cheating', 'Permanent'],
            ['Insults in voice chat', '3 days'],
        ]);
    });

    it('should say there is no template yet', async () => {
        serve({ ...STORED, templates: [] });
        await openSettings('/settings/bans');

        expect(screen.getByText(words.noTemplates)).not.toBeNull();
        expect(screen.queryByRole('list', { name: words.templates })).toBeNull();
    });

    it('should leave the templates to read for who cannot change the settings', async () => {
        serve();
        await openSettings('/settings/bans', { me: VIEWER });

        expect(screen.getByText(words.readOnly)).not.toBeNull();
        expect(listed()).toHaveLength(2);
        const cheating = named('Cheating', 'Permanent');
        for (const name of [words.addTemplate, words.removeTemplate(cheating), words.moveDown(cheating)]) {
            expect(screen.getByRole('button', { name }).matches(':disabled')).toBe(true);
        }
    });
});
