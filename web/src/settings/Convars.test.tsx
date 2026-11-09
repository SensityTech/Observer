import { QueryClient } from '@tanstack/react-query';
import { cleanup, fireEvent, screen } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { cfgConvarsQuery, convarCatalogQuery } from '../api/convars';
import type { CfgConvar } from '../api/generated/CfgConvar';
import type { CfgConvars } from '../api/generated/CfgConvars';
import type { Convar } from '../api/generated/Convar';
import type { ConvarCatalog } from '../api/generated/ConvarCatalog';
import type { ExtraArgument } from '../api/generated/ExtraArgument';
import type { StoredLaunch } from '../api/generated/StoredLaunch';
import { DEFAULT_LAUNCH, launchQuery } from '../api/launch';
import { en } from '../i18n/en';
import { openSettings } from './fixtures';

const words = en.settings.launch.convars;

const convar = (fields: Partial<Convar> & Pick<Convar, 'name'>): Convar => ({
    kind: null,
    cppType: null,
    default: null,
    defaultExpression: null,
    values: [],
    open: false,
    range: null,
    flags: [],
    platform: null,
    declared: true,
    description: null,
    sources: [{ file: 'code/components/citizen-server-impl/src/GameServer.cpp', line: 1 }],
    ...fields,
});

const CATALOG: ConvarCatalog = {
    repository: 'https://github.com/citizenfx/fivem',
    commit: '5c3065c261fb3ba82032fee6803f247198fec221',
    build: 38435,
    convars: [
        convar({
            name: 'onesync',
            kind: 'enum',
            default: 'on',
            values: [
                { value: 'on', aliases: ['true'] },
                { value: 'legacy', aliases: [] },
                { value: 'off', aliases: ['false'] },
            ],
            flags: ['internal'],
        }),
        convar({
            name: 'sv_enforceGameBuild',
            kind: 'enum',
            defaultExpression: 'xbr::GetMandatedDefaultGTA5BuildString()',
            values: [{ value: '3095', aliases: ['mpchop2'] }],
            open: true,
            flags: ['serverInfo', 'readOnly'],
        }),
        convar({ name: 'sv_pureLevel', kind: 'int', default: '0', flags: ['serverInfo'] }),
        convar({
            name: 'sv_endpoints',
            kind: 'string',
            default: '',
            description: "list of space-separated endpoints that can but don't have to include a port",
        }),
        convar({ name: 'sv_licenseKeyToken', declared: false, flags: null }),
    ],
};

const set = (name: string, value: string | null, line: number): CfgConvar => ({
    name,
    value,
    command: 'set',
    file: 'server.cfg',
    line,
});

const cfgOf = (convars: CfgConvar[], fields: Partial<CfgConvars> = {}): CfgConvars => ({
    cfgFile: 'server.cfg',
    problem: null,
    convars,
    complete: true,
    ...fields,
});

/** The Launch category with these arguments saved, the catalog and the cfg already read. */
const page = async (extraArguments: ExtraArgument[], cfg: CfgConvars | null = null) => {
    const client = new QueryClient({
        defaultOptions: { queries: { staleTime: Infinity, retry: false }, mutations: { retry: false } },
    });
    const saved: StoredLaunch = {
        settings: { ...DEFAULT_LAUNCH, dataPath: 'C:\\FXServer\\server-data', extraArguments },
        version: 1,
        legacyBuildPath: null,
    };
    client.setQueryData(launchQuery.queryKey, saved);
    client.setQueryData(convarCatalogQuery.queryKey, CATALOG);
    client.setQueryData(cfgConvarsQuery.queryKey, cfg);
    const view = await openSettings('/settings/launch', { client });
    fireEvent.click(screen.getByRole('button', { name: new RegExp(en.settings.launch.advanced) }));
    return view;
};

const names = () => screen.getAllByLabelText(en.server.argumentName) as HTMLInputElement[];
const values = () => screen.getAllByLabelText(en.server.argumentValue) as HTMLInputElement[];
const suggestions = (input: HTMLInputElement) =>
    [...(input.list?.querySelectorAll('option') ?? [])].map((option) => option.value);

describe('Convars of the launch settings', () => {
    afterEach(cleanup);

    it('should say what the sources declare of each convar, and what they do not know', async () => {
        const { getByText } = await page([
            { name: 'sv_enforceGameBuild', value: '3095' },
            { name: 'SV_ENDPOINTS', value: '203.0.113.7' },
            { name: 'sv_licenseKeyToken', value: 'x' },
            { name: 'lab_convar', value: '1' },
        ]);

        getByText(
            `${words.values('3095')} ${words.orAnyNumber} · ${words.defaultInCode} · public · set at start only`,
        );
        getByText(`text · ${words.defaultValue('')}`);
        getByText(
            words.comment("list of space-separated endpoints that can but don't have to include a port"),
        );
        getByText(words.undeclared);
        getByText(words.unknown('38435'));
        getByText(words.catalog('38435'));
    });

    it('should suggest the catalog names, and the values a convar type reads', async () => {
        await page([
            { name: 'sv_enforceGameBuild', value: '3095' },
            { name: 'sv_pureLevel', value: '1' },
        ]);

        expect(suggestions(names()[0] as HTMLInputElement)).toEqual([
            'onesync',
            'sv_enforceGameBuild',
            'sv_pureLevel',
            'sv_endpoints',
        ]);
        expect(suggestions(values()[0] as HTMLInputElement)).toEqual(['3095']);
        expect(values()[1]?.list).toBeNull();
    });

    it('should start a convar picked from the catalog at its default, and leave a typed value alone', async () => {
        await page([{ name: 'lab_convar', value: '7' }]);
        fireEvent.click(screen.getByRole('button', { name: en.server.addArgument }));

        fireEvent.change(names()[1] as HTMLInputElement, { target: { value: 'sv_pureLevel' } });
        fireEvent.change(names()[0] as HTMLInputElement, { target: { value: 'sv_pureLevel' } });

        expect(values()[1]?.value).toBe('0');
        expect(values()[0]?.value).toBe('7');
    });

    it("should say the cfg's own value applies over the command line, OneSync's included", async () => {
        const { getByText } = await page(
            [
                { name: 'sv_enforceGameBuild', value: '3095' },
                { name: 'sv_pureLevel', value: '1' },
                { name: 'rcon_unrelated', value: 'x' },
            ],
            cfgOf([
                set('sv_enforceGameBuild', '2699', 3),
                set('onesync', 'legacy', 4),
                set('SV_ENFORCEGAMEBUILD', '2802', 12),
                set('sv_pureLevel', '1', 13),
                set('rcon_unrelated', null, 14),
            ]),
        );

        getByText(words.cfgSets('server.cfg', 12, '2802'));
        getByText(words.cfgSets('server.cfg', 4, 'legacy'));
        getByText(words.cfgSetsSame('server.cfg', 13));
        getByText(words.cfgSetsSecret('server.cfg', 14));
    });

    it('should list what only the cfg sets, its secrets without their value, folded', async () => {
        const { getByText } = await page(
            [{ name: 'sv_pureLevel', value: '1' }],
            cfgOf(
                [set('sv_hostname', 'Lab', 2), set('rcon_password', null, 3), set('sv_pureLevel', '2', 4)],
                {
                    complete: false,
                },
            ),
        );

        const summary = getByText(words.inCfg(2));
        expect(summary.closest('details')?.open).toBe(false);
        getByText('sv_hostname');
        getByText(words.place('server.cfg', 3));
        getByText(words.secret);
        getByText(words.incomplete);
    });

    it('should say when the cfg could not be read', async () => {
        const { getByText } = await page([], cfgOf([], { problem: 'missing' }));

        getByText(words.unreadable('server.cfg'));
    });
});
