import { describe, expect, it } from 'vitest';

import type { Me } from '../api/generated/Me';
import { en } from '../i18n/en';
import { APPLIES, CATEGORIES, firstCategory, search } from './catalog';

const OWNER: Me = { id: 1, username: 'ada', isOwner: true, permissions: 'fffff', mustChangePassword: false };
/** Views the dashboard and manages users, and nothing else. */
const MANAGER: Me = {
    id: 2,
    username: 'kenji',
    isOwner: false,
    permissions: '81',
    mustChangePassword: false,
};
const NOBODY: Me = { id: 3, username: 'lin', isOwner: false, permissions: '0', mustChangePassword: false };

const found = (me: Me, query: string) => search(en, me, query).map(({ id }) => id);

describe('the settings catalog', () => {
    it('should name and explain every setting it lists, each once', () => {
        const ids = CATEGORIES.flatMap(({ settings }) => settings.map(({ id }) => id));

        expect(new Set(ids).size).toBe(ids.length);
        for (const id of ids) {
            expect(en.settings.items[id].label, id).not.toBe('');
            expect(en.settings.items[id].about.length, id).toBeGreaterThan(20);
        }
    });

    it('should say when a change applies for every setting saved from the page', () => {
        for (const id of [
            'onesync',
            'hangDetection',
            'timeZone',
            'keepBuilds',
            'hashRate',
            'toolAtStart',
        ] as const) {
            expect(APPLIES.get(id), id).toBeDefined();
        }
        expect(APPLIES.get('keepBuilds')).toBe('cleanup');
        expect(APPLIES.get('hangTimeout')).toBe('now');
        expect(APPLIES.get('cfgFile')).toBe('nextStart');
    });
});

describe('search', () => {
    it('should find a setting by its name or by what its explanation says, whatever the case', () => {
        expect(found(OWNER, 'OneSync')).toEqual(['onesync']);
        expect(found(OWNER, 'fingerprint')).toEqual(['hashRate', 'addressDays']);
        expect(found(OWNER, 'HASH')).toContain('hashRate');
    });

    it('should need every word typed, in any order', () => {
        expect(found(OWNER, 'time zone')).toContain('timeZone');
        expect(found(OWNER, 'zone time')).toContain('timeZone');
        expect(found(OWNER, 'zone onesync')).toEqual([]);
    });

    it('should find the settings of a category by its name', () => {
        expect(found(OWNER, 'web panel')).toEqual(['listen', 'hosts', 'trustedProxies']);
    });

    it('should leave out what the account cannot open', () => {
        expect(found(OWNER, 'password')).toContain('password');
        expect(found(MANAGER, 'onesync')).toEqual([]);
        expect(found(MANAGER, 'accounts')).toContain('accounts');
        expect(found(NOBODY, 'restart')).toEqual([]);
        expect(found(NOBODY, 'password')).toEqual(['password']);
    });

    it('should find nothing for an empty query', () => {
        expect(found(OWNER, '')).toEqual([]);
        expect(found(OWNER, '   ')).toEqual([]);
    });

    it('should ignore accents typed or written', () => {
        expect(found(OWNER, 'ÖneSync')).toEqual(['onesync']);
    });
});

describe('firstCategory', () => {
    it('should open on the first category the account may see, its own at worst', () => {
        expect(firstCategory(OWNER)).toBe('launch');
        expect(firstCategory(MANAGER)).toBe('restarts');
        expect(firstCategory(NOBODY)).toBe('account');
    });
});
