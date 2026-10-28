import { describe, expect, it } from 'vitest';

import { cleaned, countChanges, DEFAULT_LAUNCH, launchIssues } from './launch';

describe('launchIssues', () => {
    it('should keep the issues the form can place and drop the rest', () => {
        expect(
            launchIssues({
                issues: [
                    { field: 'buildPath', index: null, problem: 'notAbsolute' },
                    { field: 'argumentName', index: 2, problem: 'secret' },
                    { field: 'monitorMode', index: null, problem: 'reserved' },
                    { field: 'cfgFile', index: null, problem: 'unheard_of' },
                    { field: 'argumentValue', index: '1', problem: 'required' },
                ],
            }),
        ).toEqual([
            { field: 'buildPath', index: null, problem: 'notAbsolute' },
            { field: 'argumentName', index: 2, problem: 'secret' },
        ]);
        expect(launchIssues({})).toEqual([]);
        expect(launchIssues({ issues: 'none' })).toEqual([]);
    });
});

describe('countChanges', () => {
    it('should count each setting that differs, the arguments as one', () => {
        const saved = { ...DEFAULT_LAUNCH, buildPath: 'C:/FXServer/build' };

        expect(countChanges(saved, saved)).toBe(0);
        expect(
            countChanges(saved, {
                ...saved,
                onesync: 'legacy',
                extraArguments: [
                    { name: 'sv_hostname', value: 'Lab' },
                    { name: 'sv_maxclients', value: '48' },
                ],
            }),
        ).toBe(2);
    });
});

describe('cleaned', () => {
    it('should trim the paths and names a paste leaves spaces around, never a value', () => {
        expect(
            cleaned({
                ...DEFAULT_LAUNCH,
                buildPath: '  C:/FXServer/build ',
                cfgFile: 'server.cfg\t',
                extraArguments: [{ name: ' sv_hostname ', value: ' Lab ' }],
            }),
        ).toEqual({
            ...DEFAULT_LAUNCH,
            buildPath: 'C:/FXServer/build',
            cfgFile: 'server.cfg',
            extraArguments: [{ name: 'sv_hostname', value: ' Lab ' }],
        });
    });
});
