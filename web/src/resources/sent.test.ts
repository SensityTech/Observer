import { describe, expect, it } from 'vitest';

import type { ResourceRow } from '../api/generated/ResourceRow';
import { answer, drop, NOTHING_SENT, RESCAN, seen, send, underWay } from './sent';

const row = (name: string, changes: Partial<ResourceRow> = {}): ResourceRow => ({
    name,
    folder: '',
    origin: 'server',
    version: null,
    state: 'started',
    failedAt: null,
    warnings: null,
    dependants: [],
    controllable: true,
    last: null,
    asked: null,
    ...changes,
});

describe('the orders a page sent', () => {
    it('hold from the click until the first view after the answer', () => {
        const clicked = send(NOTHING_SENT, 'ox_lib', 'restart');
        expect(underWay(row('ox_lib'), clicked)).toBe('restart');
        expect(underWay(row('taxi'), clicked)).toBeNull();

        // The view that shows the order may come before the answer: the order is still this page's.
        const early = seen(clicked);
        expect(early).toBe(clicked);

        const answered = answer(early, 'ox_lib');
        expect(underWay(row('ox_lib'), answered)).toBe('restart');
        expect(seen(answered).size).toBe(0);
    });

    it('go when the API refused them or FXServer never got them', () => {
        const two = send(send(NOTHING_SENT, 'ox_lib', 'stop'), 'taxi', 'start');

        const left = drop(two, 'ox_lib');

        expect([...left.keys()]).toEqual(['taxi']);
        expect(drop(left, 'ox_lib')).toBe(left);
        expect(answer(left, 'ox_lib')).toBe(left);
    });

    it('give way to what the list says FXServer has not answered, whoever asked', () => {
        const asked = row('police', { asked: { kind: 'restarted', by: 'maxime' } });
        expect(underWay(asked, NOTHING_SENT)).toBe('restart');
        expect(underWay(row('taxi', { asked: { kind: 'started', by: 'ada' } }), NOTHING_SENT)).toBe('start');
        expect(underWay(row('chat', { asked: { kind: 'stopped', by: 'ada' } }), NOTHING_SENT)).toBe('stop');

        // This page's own order shows first: the list may still say the one before it.
        expect(underWay(asked, send(NOTHING_SENT, 'police', 'stop'))).toBe('stop');
    });

    it('keep a rescan apart from every resource', () => {
        const rescanning = send(NOTHING_SENT, RESCAN, 'rescan');

        expect(rescanning.has(RESCAN)).toBe(true);
        expect(underWay(row('ox_lib'), rescanning)).toBeNull();
        expect(seen(answer(rescanning, RESCAN)).has(RESCAN)).toBe(false);
    });
});
