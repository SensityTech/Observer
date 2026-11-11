import { describe, expect, it } from 'vitest';

import type { ResourceRow } from '../api/generated/ResourceRow';
import { en } from '../i18n/en';
import { formatDuration } from '../i18n/format';
import { changeLine, warningLines } from './words';

const format = { duration: (seconds: number) => formatDuration('en', seconds) };
const NOW = Date.parse('2026-11-11T14:30:00Z');

const row = (changes: Partial<ResourceRow> = {}): ResourceRow => ({
    name: 'ox_target',
    folder: '[core]/[ox]',
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

const line = (changes: Partial<ResourceRow>, order: Parameters<typeof changeLine>[4] = null) =>
    changeLine(en, format, row(changes), NOW, order);

describe('what a row says under its state', () => {
    it('should say nothing of a resource nothing happened to since the cfg ran', () => {
        expect(line({})).toBeNull();
    });

    it('should say what last happened, who asked for it and the resource it stopped with', () => {
        const last = { at: '2026-11-11T14:28:00Z', kind: 'restarted', by: null, with: null } as const;
        expect(line({ last })).toBe('restarted 2 min ago');
        expect(line({ last: { ...last, at: '2026-11-11T14:29:40Z', by: 'maxime' } })).toBe(
            'restarted by maxime, just now',
        );
        expect(line({ state: 'stopped', last: { ...last, kind: 'stopped', with: 'ox_lib' } })).toBe(
            'stopped with ox_lib, 2 min ago',
        );
        expect(line({ last: { ...last, by: 'ada', with: 'ox_lib' } })).toBe(
            'restarted with ox_lib by ada, 2 min ago',
        );
        expect(line({ last: { ...last, kind: 'started', at: '2026-11-11T11:28:00Z' } })).toBe(
            'started 3h 2m ago',
        );
    });

    it('should say that FXServer could not start a resource, until it starts', () => {
        const failed = { state: 'stopped', failedAt: '2026-11-11T13:59:00Z' } as const;
        const stopped = { at: '2026-11-11T13:00:00Z', kind: 'stopped', by: 'ada', with: null } as const;
        expect(line({ ...failed, last: stopped })).toBe('FXServer could not start it, 31 min ago');
        expect(line({ ...failed, state: 'started', last: { ...stopped, kind: 'started' } })).toBe(
            'started by ada, 1h 30m ago',
        );
    });

    it('should say first the order FXServer has not answered, and who asked for it', () => {
        const last = { at: '2026-11-11T14:28:00Z', kind: 'stopped', by: null, with: null } as const;
        expect(line({ last }, { verb: 'restart', by: 'maxime' })).toBe('restart asked by maxime');
        expect(line({ state: 'stopped' }, { verb: 'start', by: 'ada' })).toBe('start asked by ada');
        expect(line({}, { verb: 'stop', by: 'ada' })).toBe('stop asked by ada');
    });
});

describe('what FXServer warned of as it started a resource', () => {
    it('should tell the heavy files, the oversized among them, the missing ones and the rest', () => {
        expect(warningLines(en, { count: 3, heavy: 2, oversized: 1, missing: 1 })).toEqual([
            '2 streamed files are heavy, 1 of them oversized',
            '1 file its manifest names is missing',
        ]);
        expect(warningLines(en, { count: 4, heavy: 1, oversized: 1, missing: 0 })).toEqual([
            '1 streamed file is oversized',
            '3 other warnings, in the console',
        ]);
        expect(warningLines(en, { count: 2, heavy: 0, oversized: 0, missing: 2 })).toEqual([
            '2 files its manifest names are missing',
        ]);
        expect(warningLines(en, { count: 1, heavy: 0, oversized: 0, missing: 0 })).toEqual([
            '1 other warning, in the console',
        ]);
    });
});
