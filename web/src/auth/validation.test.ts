import { describe, expect, it } from 'vitest';

import { confirmationProblem, newPasswordProblem, passwordProblem, usernameProblem } from './validation';

describe('usernameProblem', () => {
    it('should accept 3 to 32 letters, digits, dots, dashes and underscores', () => {
        expect(usernameProblem('ada')).toBeNull();
        expect(usernameProblem('kenji.mod-2_eu')).toBeNull();
        expect(usernameProblem('a'.repeat(32))).toBeNull();
    });

    it('should refuse what the API refuses, with its code', () => {
        expect(usernameProblem('ab')).toEqual({ code: 'invalid_username' });
        expect(usernameProblem('a'.repeat(33))).toEqual({ code: 'invalid_username' });
        expect(usernameProblem('ken ji')).toEqual({ code: 'invalid_username' });
        expect(usernameProblem('kénji')).toEqual({ code: 'invalid_username' });
    });

    it('should ask for a name when the field is empty', () => {
        expect(usernameProblem('')).toEqual({ code: 'required' });
    });
});

describe('passwordProblem', () => {
    it('should count characters, not bytes, for the minimum of 12', () => {
        expect(passwordProblem('correct-hors')).toBeNull();
        expect(passwordProblem('short-pass1')).toEqual({ code: 'password_too_short', params: { min: 12 } });
        // Twelve characters of two bytes each: long enough.
        expect(passwordProblem('é'.repeat(12))).toBeNull();
    });

    it('should count bytes for the maximum of 1024', () => {
        expect(passwordProblem('a'.repeat(1024))).toBeNull();
        expect(passwordProblem('é'.repeat(513))).toEqual({
            code: 'password_too_long',
            params: { max: 1024 },
        });
    });

    it('should refuse a new password equal to the current one', () => {
        expect(newPasswordProblem('the-old-password', 'the-old-password')).toEqual({
            code: 'password_unchanged',
        });
        expect(newPasswordProblem('the-old-password', 'a-new-password!')).toBeNull();
    });
});

describe('confirmationProblem', () => {
    it('should want the confirmation to repeat the password exactly', () => {
        expect(confirmationProblem('correct-horse', 'correct-horse')).toBeNull();
        expect(confirmationProblem('correct-horse', 'correct-horsE')).toEqual({ code: 'passwords_differ' });
        expect(confirmationProblem('correct-horse', '')).toEqual({ code: 'required' });
    });
});
