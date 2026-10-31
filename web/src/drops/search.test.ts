import { defaultParseSearch, defaultStringifySearch } from '@tanstack/react-router';
import { describe, expect, it } from 'vitest';

import {
    chartShown,
    filtered,
    logFilters,
    logSearch,
    parseDropsSearch,
    rangeCovering,
    withFilters,
} from './search';

/** The address as the router reads it, then as the page reads that. */
const read = (address: string) => parseDropsSearch(defaultParseSearch(address));

describe('parseDropsSearch', () => {
    it('should read every filter the address holds, numbers as numbers and a search as text', () => {
        expect(
            read(
                '?range=30d&category=crash&resource=afk_kick&signature=11&run=41&player=Ruby&profile=7&group=7&chart=timeout',
            ),
        ).toEqual({
            range: '30d',
            chart: 'timeout',
            category: 'crash',
            resource: 'afk_kick',
            signature: 11,
            run: 41,
            player: 'Ruby',
            profile: 7,
            group: 7,
        });
    });

    it('should leave out what it does not know, and a kind that is not its category one', () => {
        expect(read('?range=1y&category=boom&run=-3&signature=abc&player=%20%20&chart=pie')).toEqual({});
        expect(read('?category=timeout&kind=resource')).toEqual({ category: 'timeout' });
        expect(read('?category=kick&kind=resource')).toEqual({ category: 'kick', kind: 'resource' });
    });

    it('should keep a player search of digits as text, as a licence', () => {
        expect(read('?player=123456')).toEqual({ player: '123456' });
    });
});

describe('logSearch', () => {
    it('should write the range, then each filter set, then where the page starts', () => {
        expect(logSearch({ range: '7d' })).toBe('?range=7d');
        expect(
            logSearch(
                {
                    range: '24h',
                    category: 'kick',
                    kind: 'resource',
                    resource: 'afk_kick',
                    player: 'discord:123',
                },
                '1761760800000.899',
            ),
        ).toBe(
            '?range=24h&category=kick&kind=resource&resource=afk_kick&player=discord%3A123&before=1761760800000.899',
        );
    });

    it('should take the address to the API and back to the same address', () => {
        const address = '?range=24h&category=crash&signature=11&run=41&player=license%3A5f3c&profile=7';
        const search = read(address);
        const filters = logFilters(search);

        expect(logSearch(filters)).toBe(
            '?range=24h&category=crash&signature=11&run=41&player=license%3A5f3c&profile=7',
        );
        expect(defaultStringifySearch(search)).toBe(address);
        expect(read(defaultStringifySearch(search))).toEqual(search);
    });

    it('should give the log the default range when the address names none', () => {
        expect(logFilters(read('?run=41'))).toEqual({ range: '7d', run: 41 });
        expect(filtered(read('?range=30d'))).toBe(false);
        expect(filtered(read('?run=41'))).toBe(true);
    });
});

describe('withFilters', () => {
    it('should replace the log filters and keep the range and the chart', () => {
        const search = read('?range=30d&chart=kick&category=crash&run=41');
        expect(withFilters(search, { resource: 'afk_kick' })).toEqual({
            range: '30d',
            chart: 'kick',
            resource: 'afk_kick',
        });
        expect(withFilters(search, {})).toEqual({ range: '30d', chart: 'kick' });
    });
});

describe('chartShown', () => {
    it('should follow the category the log is narrowed to unless a chart was chosen', () => {
        expect(chartShown(read('?category=timeout'))).toBe('timeout');
        expect(chartShown(read('?category=unknown'))).toBe('unexpected');
        expect(chartShown(read('?category=quit'))).toBe('unexpected');
        expect(chartShown(read('?category=timeout&chart=crash'))).toBe('crash');
    });
});

describe('rangeCovering', () => {
    it('should pick the shortest range that still holds the moment', () => {
        const now = Date.parse('2026-10-30T19:00:00Z');
        expect(rangeCovering('2026-10-30T12:00:00Z', now)).toBe('24h');
        expect(rangeCovering('2026-10-27T12:00:00Z', now)).toBe('7d');
        expect(rangeCovering('2026-10-10T12:00:00Z', now)).toBe('30d');
        expect(rangeCovering('2026-08-10T12:00:00Z', now)).toBe('90d');
    });
});
