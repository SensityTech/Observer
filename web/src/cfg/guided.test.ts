import { describe, expect, it } from 'vitest';

import type { CfgOutline } from '../api/generated/CfgOutline';
import {
    UNTOUCHED,
    applyGuided,
    pairedEndpoints,
    resourceName,
    settingOf,
    settingsOf,
    toggled,
    touched,
    writable,
    type Guided,
} from './guided';

const TEXT = [
    'endpoint_add_tcp "0.0.0.0:30120"',
    'endpoint_add_udp "0.0.0.0:30120"',
    '# What players see',
    'sv_hostname "Lab 😀" // the list',
    'sets tags "roleplay, fr"; sv_maxclients 48',
    'sv_licenseKey <<secret-1>>',
    'sv_hostname "Kept"',
    '',
    'ensure chat',
    '  #ensure [xmas]',
    'ensure a; ensure b',
    '',
].join('\n');

/** As the server outlines the text above. */
const OUTLINE: CfgOutline = {
    settings: [
        {
            line: 4,
            command: null,
            name: 'sv_hostname',
            value: 'Lab 😀',
            start: 12,
            end: 20,
            alone: true,
        },
        {
            line: 5,
            command: 'sets',
            name: 'tags',
            value: 'roleplay, fr',
            start: 10,
            end: 24,
            alone: false,
        },
        {
            line: 5,
            command: null,
            name: 'sv_maxclients',
            value: '48',
            start: 40,
            end: 42,
            alone: false,
        },
        { line: 6, command: null, name: 'sv_licenseKey', value: null, start: 14, end: 26, alone: true },
        { line: 7, command: null, name: 'SV_HOSTNAME', value: 'Kept', start: 12, end: 18, alone: true },
    ],
    endpoints: [
        { line: 1, protocol: 'tcp', address: '0.0.0.0:30120', start: 17, end: 32, alone: true },
        { line: 2, protocol: 'udp', address: '0.0.0.0:30120', start: 17, end: 32, alone: true },
    ],
    resources: [
        { line: 9, name: 'chat', enabled: true, alone: true, missing: false },
        { line: 10, name: '[xmas]', enabled: false, alone: true, missing: false },
        { line: 11, name: 'a', enabled: true, alone: false, missing: true },
        { line: 11, name: 'b', enabled: true, alone: false, missing: true },
    ],
    truncated: false,
};

const guided = (changes: Partial<Guided>): Guided => ({ ...UNTOUCHED, ...changes });
const line = (text: string, number: number) => text.split('\n')[number - 1];

describe('applyGuided', () => {
    it('should leave a text it has nothing to write into as it is', () => {
        expect(applyGuided(TEXT, OUTLINE, UNTOUCHED)).toBe(TEXT);
        expect(touched(UNTOUCHED)).toBe(false);
        expect(touched(guided({ started: { 9: false } }))).toBe(true);
    });

    it('should write a value in place of the one the last line of its convar held', () => {
        const text = applyGuided(TEXT, OUTLINE, guided({ values: { sv_hostname: 'Café \\ RP' } }));

        expect(line(text, 7)).toBe('sv_hostname "Café \\\\ RP"');
        expect(line(text, 4)).toBe(line(TEXT, 4));
        expect(settingOf(OUTLINE, 'sv_hostname')?.line).toBe(7);
        expect(settingsOf(OUTLINE, 'sv_hostname')).toBe(2);
    });

    it('should write two values of one line each in its place, a number without quotes', () => {
        const text = applyGuided(
            TEXT,
            OUTLINE,
            guided({ values: { tags: 'roleplay, fr, serious', sv_maxclients: '128' } }),
        );

        expect(line(text, 5)).toBe('sets tags "roleplay, fr, serious"; sv_maxclients 128');
    });

    it('should write a new secret over the mark of the one it replaces', () => {
        const text = applyGuided(TEXT, OUTLINE, guided({ values: { sv_licenseKey: 'cfxk_new' } }));

        expect(line(text, 6)).toBe('sv_licenseKey "cfxk_new"');
    });

    it('should write the game address to both endpoints', () => {
        const text = applyGuided(TEXT, OUTLINE, guided({ endpoint: '0.0.0.0:30125' }));

        expect([line(text, 1), line(text, 2)]).toEqual([
            'endpoint_add_tcp "0.0.0.0:30125"',
            'endpoint_add_udp "0.0.0.0:30125"',
        ]);
        expect(pairedEndpoints(OUTLINE)).not.toBeNull();
        expect(
            pairedEndpoints({ ...OUTLINE, endpoints: [OUTLINE.endpoints[0]!, OUTLINE.endpoints[0]!] }),
        ).toBeNull();
        expect(pairedEndpoints({ ...OUTLINE, endpoints: OUTLINE.endpoints.slice(0, 1) })).toBeNull();
    });

    it('should add a line for a convar the file does not set, after the last one it has a field for', () => {
        const text = applyGuided(
            TEXT,
            OUTLINE,
            guided({ values: { sv_projectName: 'Lab', sv_enforceGameBuild: '3258', locale: 'fr-FR' } }),
        );

        expect(text.split('\n').slice(6, 11)).toEqual([
            'sv_hostname "Kept"',
            'sets sv_projectName "Lab"',
            'sets locale "fr-FR"',
            'sv_enforceGameBuild 3258',
            '',
        ]);
        expect(
            applyGuided(
                'ensure chat\n',
                { ...OUTLINE, settings: [], endpoints: [] },
                guided({ values: { tags: 'fr' } }),
            ),
        ).toBe('sets tags "fr"\nensure chat\n');
    });

    it('should comment out the line of a resource turned off and put back one turned on', () => {
        const text = applyGuided(TEXT, OUTLINE, guided({ started: { 9: false, 10: true, 11: false } }));

        expect([line(text, 9), line(text, 10), line(text, 11)]).toEqual([
            '#ensure chat',
            '  ensure [xmas]',
            'ensure a; ensure b',
        ]);
        expect(toggled('// start chat # later', true)).toBe('start chat # later');
        expect(toggled('\tensure chat', false)).toBe('\t#ensure chat');
    });

    it('should start the resources added after the last line that starts one, in their order', () => {
        const text = applyGuided(TEXT, OUTLINE, guided({ added: ['maps', '[jobs]', 'not a name'] }));

        expect(text.split('\n').slice(10)).toEqual([
            'ensure a; ensure b',
            'ensure maps',
            'ensure [jobs]',
            '',
        ]);
        expect(
            applyGuided('sv_hostname "x"\n', { ...OUTLINE, resources: [] }, guided({ added: ['chat'] })),
        ).toBe('sv_hostname "x"\nensure chat\n');
        expect(
            applyGuided('sv_hostname "x"', { ...OUTLINE, resources: [] }, guided({ added: ['chat'] })),
        ).toBe('sv_hostname "x"\nensure chat');
    });

    it('should never write a value FXServer would cut', () => {
        const text = applyGuided(
            TEXT,
            OUTLINE,
            guided({ values: { sv_hostname: 'a"; quit' }, endpoint: 'x\ny' }),
        );

        expect(text).toBe(TEXT);
        expect([writable('a"b'), writable('a\nb'), writable('a; b \\ c')]).toEqual([false, false, true]);
        expect(['chat', '[maps]', 'a b', 'a;b', 'a//b', '#a', ''].map(resourceName)).toEqual([
            true,
            true,
            false,
            false,
            false,
            false,
            false,
        ]);
    });
});
