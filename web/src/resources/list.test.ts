import { describe, expect, it } from 'vitest';

import type { ResourceRow } from '../api/generated/ResourceRow';
import { countByFilter, didNotStart, groupByFolder, passes } from './list';

const row = (name: string, folder: string, changes: Partial<ResourceRow> = {}): ResourceRow => ({
    name,
    folder,
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

const WARNED = { count: 2, heavy: 1, oversized: 0, missing: 1 };

const ROWS = [
    row('chat', '', { origin: 'build' }),
    row('docks', '[maps]', { warnings: WARNED }),
    row('loadscreen', ''),
    row('mechanic', '[jobs]', { state: 'stopped', failedAt: '2026-11-11T14:02:00Z' }),
    row('monitor', '', { origin: 'build', state: 'stopped' }),
    row('ox_lib', '[core]'),
    row('ox_target', '[core]/[ox]', { state: 'stopped' }),
    row('Police', '[jobs]', { failedAt: '2026-11-11T13:00:00Z' }),
    row('taxi', '[jobs]', { state: 'stopped', warnings: WARNED }),
    row('webpack', '[build]', { origin: 'build', state: 'starting' }),
];

const names = (rows: readonly ResourceRow[]) => rows.map((each) => each.name);

describe('the filters of the resources', () => {
    it('take a resource FXServer could not start for one that did not start, until it starts', () => {
        expect(names(ROWS.filter(didNotStart))).toEqual(['mechanic']);
        expect(names(ROWS.filter((each) => passes(each, 'stopped')))).toEqual([
            'monitor',
            'ox_target',
            'taxi',
        ]);
    });

    it('count what each of them shows', () => {
        expect(countByFilter(ROWS)).toEqual({ all: 10, started: 5, stopped: 3, failed: 1, warned: 2 });
        expect(countByFilter([])).toEqual({ all: 0, started: 0, stopped: 0, failed: 0, warned: 0 });
    });
});

describe('the resources by folder', () => {
    it("come root first, then the server's categories by name, then the build's own", () => {
        const groups = groupByFolder(ROWS, 'all', '');

        expect(groups.map((group) => [group.folder, names(group.rows)])).toEqual([
            ['resources', ['loadscreen']],
            ['[core]', ['ox_lib']],
            ['[core]/[ox]', ['ox_target']],
            ['[jobs]', ['mechanic', 'Police', 'taxi']],
            ['[maps]', ['docks']],
            ['system_resources', ['chat', 'monitor']],
            ['system_resources/[build]', ['webpack']],
        ]);
        expect(new Set(groups.map((group) => group.key)).size).toBe(groups.length);
    });

    it('count every resource of a folder, whatever the filter and the search show of it', () => {
        const groups = groupByFolder(ROWS, 'stopped', '');

        expect(groups.map((group) => group.folder)).toEqual(['[core]/[ox]', '[jobs]', 'system_resources']);
        const jobs = groups[1];
        expect(jobs).toMatchObject({ started: 1, total: 3 });
        expect(names(jobs?.rows ?? [])).toEqual(['taxi']);
    });

    it('are searched by name, without regard to case, and a folder with no match is left out', () => {
        const found = groupByFolder(ROWS, 'all', '  POL ');

        expect(found.map((group) => [group.folder, names(group.rows)])).toEqual([['[jobs]', ['Police']]]);
        expect(groupByFolder(ROWS, 'failed', 'pol')).toEqual([]);
        expect(groupByFolder(ROWS, 'warned', '').flatMap((group) => names(group.rows))).toEqual([
            'taxi',
            'docks',
        ]);
    });
});
