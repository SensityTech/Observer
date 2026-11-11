import { cleanup, fireEvent } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { en } from '../i18n/en';
import { openSettings, settle } from './fixtures';

const words = en.settings.bans;
const bar = en.settings.bar;
const label = en.settings.items.kickMessage.label;
const KICKS = '/servers/default/kicks/settings';
const STORED = 'You were kicked.\nReason: {reason}';

/** Observer's answers by method and route; every request made is kept, with its body. */
const serve = (replies: Record<string, unknown>) => {
    const asked: { route: string; body: unknown }[] = [];
    vi.stubGlobal(
        'fetch',
        vi.fn(async (input: string, init: RequestInit) => {
            const route = `${init.method ?? 'GET'} ${new URL(input, 'http://localhost').pathname.replace(/^\/api/, '')}`;
            asked.push({ route, body: typeof init.body === 'string' ? JSON.parse(init.body) : undefined });
            const reply = replies[route];
            return Response.json(reply ?? { code: 'not_found', params: {} }, {
                status: reply === undefined ? 404 : 200,
            });
        }),
    );
    return asked;
};

/** The bans' category, once its kick message is read; `puts` are the saves asked of Observer. */
const open = async () => {
    const asked = serve({
        'GET /servers/default/bans/settings': { message: 'Banned: {reason}', tokenMatches: 1, templates: [] },
        [`GET ${KICKS}`]: { message: STORED },
        [`PUT ${KICKS}`]: { message: 'Kicked by {by}: {reason}' },
    });
    const view = await openSettings('/settings/bans');
    const field = (await view.findByLabelText(label)) as HTMLTextAreaElement;
    return { ...view, field, puts: () => asked.filter(({ route }) => route.startsWith('PUT')) };
};

describe('the kick message, among the bans settings', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should show the message as a kicked player reads it, with an example reason', async () => {
        const { field, getByText } = await open();

        expect(field.value).toBe(STORED);
        expect(getByText(`You were kicked. Reason: ${words.kickExample.reason}`)).not.toBeNull();
    });

    it('should save a new message through the category bar, alone, and say when it applies', async () => {
        const { field, getByRole, getByText, findByText, puts } = await open();

        fireEvent.change(field, { target: { value: 'Kicked by {by}: {reason}' } });
        const preview = getByText(`Kicked by ${words.kickExample.by}: ${words.kickExample.reason}`);
        const counted = getByText(bar.changes(1));
        fireEvent.click(getByRole('button', { name: bar.save }));

        expect(await findByText(words.kickSaved)).not.toBeNull();
        expect(preview).not.toBeNull();
        expect(counted).not.toBeNull();
        expect(puts()).toEqual([{ route: `PUT ${KICKS}`, body: { message: 'Kicked by {by}: {reason}' } }]);
    });

    it('should refuse an empty message before asking Observer', async () => {
        const { field, getByRole, getByText, puts } = await open();

        fireEvent.change(field, { target: { value: '  \n ' } });
        fireEvent.click(getByRole('button', { name: bar.save }));
        await settle();

        expect(getByText(words.kickMessageMissing)).not.toBeNull();
        expect(puts()).toEqual([]);
    });

    it('should insert a placeholder where the cursor stands', async () => {
        const { field, getAllByRole } = await open();

        fireEvent.change(field, { target: { value: 'Kicked by .' } });
        field.setSelectionRange(10, 10);
        // The refusal message above offers `{by}` too: the kick message's is the last.
        fireEvent.click(getAllByRole('button', { name: '{by}' }).at(-1)!);

        expect(field.value).toBe('Kicked by {by}.');
    });
});
