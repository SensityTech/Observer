import { describe, expect, it } from 'vitest';

import { ARTIFACTS } from '../artifacts/fixtures';
import {
    defaultMode,
    needsKvpConfirmation,
    protectionsOf,
    secondsLeft,
    settingsIssues,
    stepsOf,
} from './artifacts';
import type { ArtifactsStatus } from './generated/ArtifactsStatus';

const STATUS = structuredClone(ARTIFACTS) as unknown as ArtifactsStatus;

describe('stepsOf', () => {
    it('should list a download and an extraction for an install, a copy for an import', () => {
        expect(stepsOf('install')).toEqual([
            'preCheck',
            'download',
            'verify',
            'extract',
            'validate',
            'commit',
        ]);
        expect(stepsOf('import')).toEqual(['preCheck', 'copy', 'validate', 'commit']);
    });
});

describe('secondsLeft', () => {
    it('should round the time left up, and know none before a rate', () => {
        expect(secondsLeft({ received: 41_200_000, total: 63_500_000, rate: 4_200_000 })).toBe(6);
        expect(secondsLeft({ received: 0, total: 63_500_000, rate: 0 })).toBeNull();
        expect(secondsLeft({ received: 63_500_000, total: 63_500_000, rate: 1 })).toBe(0);
    });
});

describe('protectionsOf', () => {
    it('should name every reason a build is in use, and none for the others', () => {
        expect(protectionsOf(STATUS, 37900)).toEqual(['selected', 'running', 'probation']);
        expect(protectionsOf(STATUS, 37950)).toEqual(['pending']);
        expect(protectionsOf(STATUS, 35245)).toEqual(['lastHealthy']);
        expect(protectionsOf(STATUS, 37000)).toEqual([]);
    });
});

describe('needsKvpConfirmation', () => {
    it('should ask for it only below 6449 once a build from 6449 ran', () => {
        expect(needsKvpConfirmation(STATUS, 6448, 'nextStart')).toBe(true);
        expect(needsKvpConfirmation(STATUS, 6449, 'nextStart')).toBe(false);
        expect(needsKvpConfirmation({ ...STATUS, highestRun: 6448 }, 5848, 'nextStart')).toBe(false);
        expect(needsKvpConfirmation({ ...STATUS, highestRun: null }, 5848, 'nextStart')).toBe(false);
    });

    it('should count the selected build as run when the build waits for the scheduled restart', () => {
        const notRunYet = { ...STATUS, highestRun: null, selected: 7000 };

        expect(needsKvpConfirmation(notRunYet, 6000, 'scheduledRestart')).toBe(true);
        expect(needsKvpConfirmation(notRunYet, 6000, 'nextStart')).toBe(false);
    });
});

describe('defaultMode', () => {
    it('should wait for the scheduled restart of a running server, and never restart by itself', () => {
        expect(defaultMode(true, true)).toBe('scheduledRestart');
        expect(defaultMode(true, false)).toBe('nextStart');
        expect(defaultMode(false, true)).toBe('nextStart');
    });
});

describe('settingsIssues', () => {
    it('should keep the issues the form can place and drop the rest', () => {
        expect(
            settingsIssues({
                issues: [
                    { field: 'keepBuilds', index: null, problem: 'outOfRange' },
                    { field: 'branches', index: 2, problem: 'master' },
                    { field: 'retention', index: null, problem: 'outOfRange' },
                    { field: 'branches', index: '1', problem: 'invalid' },
                ],
            }),
        ).toEqual([
            { field: 'keepBuilds', index: null, problem: 'outOfRange' },
            { field: 'branches', index: 2, problem: 'master' },
        ]);
        expect(settingsIssues({})).toEqual([]);
    });
});
