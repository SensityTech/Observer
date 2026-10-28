import { describe, expect, it } from 'vitest';

import { countChanges, DEFAULT_WARNINGS, scheduleIssues } from './schedule';

describe('scheduleIssues', () => {
    it('should keep the issues the panel can place and drop the rest', () => {
        const issues = scheduleIssues({
            issues: [
                { field: 'time', index: 1, problem: 'tooClose' },
                { field: 'timeZone', index: null, problem: 'unknownTimeZone' },
                { field: 'weekday', index: null, problem: 'required' },
                { field: 'warning', index: 0, problem: 'bored' },
                { field: 'time', index: '1', problem: 'duplicate' },
                'times',
            ],
        });

        expect(issues).toEqual([
            { field: 'time', index: 1, problem: 'tooClose' },
            { field: 'timeZone', index: null, problem: 'unknownTimeZone' },
        ]);
        expect(scheduleIssues({})).toEqual([]);
    });
});

describe('countChanges', () => {
    it('should count each setting once, a list included', () => {
        const saved = {
            enabled: false,
            mode: 'fixedTimes' as const,
            times: ['04:00'],
            intervalMinutes: 360,
            timeZone: null,
            warnings: DEFAULT_WARNINGS,
            skipAfterStart: 120,
        };

        expect(countChanges(saved, saved)).toBe(0);
        expect(countChanges(saved, { ...saved, enabled: true, times: ['04:00', '16:00', '22:00'] })).toBe(2);
    });
});
