import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { EntityItem } from '../api/generated/EntityItem';
import type { EntityResult } from '../api/generated/EntityResult';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { DeleteDialog } from './DeleteDialog';

const words = en.entities.dialog;

const item = (netId: number, changes: Partial<EntityItem> = {}): EntityItem => ({
    netId,
    serial: netId * 10,
    kind: 'vehicle',
    class: 'car',
    model: 0x3c4e2113,
    x: 0,
    y: 0,
    z: 0,
    owned: false,
    owner: null,
    ownerName: null,
    bucket: 0,
    population: 5,
    resource: null,
    resourcePending: false,
    occupied: false,
    ...changes,
});

/** The dialog over `items`, the server answering a deletion with `reply`. */
const open = (items: EntityItem[], reply: () => Response = () => Response.json({ results: [] })) => {
    const fetch = vi.fn(async () => reply());
    vi.stubGlobal('fetch', fetch);
    const done: EntityResult[][] = [];
    render(
        <I18nProvider>
            <QueryClientProvider client={new QueryClient()}>
                <DeleteDialog
                    items={items}
                    run="3-52100"
                    limits={{ typedConfirmationFrom: 20 }}
                    onClose={() => {}}
                    onDone={(results) => done.push(results)}
                />
            </QueryClientProvider>
        </I18nProvider>,
    );
    return { fetch, done };
};

describe('DeleteDialog', () => {
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

    it('should say what a selection is made of and what may still expect it', () => {
        open([
            ...Array.from({ length: 3 }, (_, index) =>
                item(index + 1, { resource: 'qb-garages', population: 7, owned: true }),
            ),
            item(9, { kind: 'object', class: null, model: 0xe0faf6d2 }),
        ]);

        expect(screen.getByText(words.title(4))).toBeTruthy();
        expect(screen.getByText('0x3C4E2113')).toBeTruthy();
        expect(screen.getByText(words.times(3))).toBeTruthy();
        expect(screen.getByText(words.scripted(3))).toBeTruthy();
        expect(screen.getByText(words.owned(3))).toBeTruthy();
        // Under twenty, no word to type.
        expect(screen.queryByRole('textbox')).toBeNull();
        expect(screen.getByRole<HTMLButtonElement>('button', { name: words.confirm(4) }).disabled).toBe(
            false,
        );
    });

    it('should name the one entity and the resource that created it', () => {
        open([item(1042, { resource: 'qb-garages', population: 7 })]);

        expect(screen.getByText(words.titleOne('Car'))).toBeTruthy();
        expect(screen.getByText(words.createdBy('qb-garages'))).toBeTruthy();
        expect(screen.getByRole('button', { name: words.confirmOne })).toBeTruthy();
    });

    it('should ask for the word before deleting twenty entities or more', async () => {
        const { fetch, done } = open(
            Array.from({ length: 20 }, (_, index) => item(index + 1)),
            () => Response.json({ results: [{ netId: 1, outcome: 'deleted' }] }),
        );
        const confirm = screen.getByRole<HTMLButtonElement>('button', { name: words.confirm(20) });

        expect(confirm.disabled).toBe(true);
        fireEvent.change(screen.getByLabelText(words.type(words.word, 20)), { target: { value: 'delete' } });
        expect(confirm.disabled).toBe(true);
        fireEvent.change(screen.getByLabelText(words.type(words.word, 20)), { target: { value: 'DELETE' } });
        expect(confirm.disabled).toBe(false);

        fireEvent.click(confirm);
        await waitFor(() => expect(done).toEqual([[{ netId: 1, outcome: 'deleted' }]]));
        expect(fetch).toHaveBeenCalledTimes(1);
    });

    it('should stay open and say why when the server refuses the deletion', async () => {
        const { done } = open([item(1)], () =>
            Response.json({ code: 'entities_out_of_date', params: {} }, { status: 409 }),
        );

        fireEvent.click(screen.getByRole('button', { name: words.confirmOne }));

        await waitFor(() => expect(screen.getByText(en.errors.entities_out_of_date)).toBeTruthy());
        expect(done).toEqual([]);
    });
});
