import { describe, expect, it } from 'vitest';

import { ApiError } from '../api/client';
import { en } from './en';
import { errorMessage, problemMessage } from './errors';

describe('errorMessage', () => {
    it('should word a plain error code from the catalog', () => {
        expect(errorMessage(en, new ApiError(401, 'invalid_credentials'))).toBe(
            'Wrong username or password.',
        );
        expect(errorMessage(en, new ApiError(409, 'conflict'))).toBe(en.errors.conflict);
    });

    it('should fill the message with the error parameters', () => {
        expect(errorMessage(en, new ApiError(429, 'too_many_attempts', { retryAfter: 58 }))).toBe(
            'Too many attempts. Try again in 58 s.',
        );
        expect(errorMessage(en, new ApiError(429, 'too_many_attempts', { retryAfter: 840 }))).toBe(
            'Too many attempts. Try again in 14 min.',
        );
        expect(errorMessage(en, new ApiError(400, 'password_too_short', { min: 12 }))).toBe(
            'Use at least 12 characters.',
        );
    });

    it('should name permissions from their catalog key or their hexadecimal set', () => {
        expect(errorMessage(en, new ApiError(403, 'forbidden', { permission: 'auditView' }))).toBe(
            'Your account does not hold the permission this needs: View the audit log.',
        );
        expect(errorMessage(en, new ApiError(403, 'escalation', { permissions: '600' }))).toBe(
            'You can only grant permissions you hold yourself, not View settings and Change settings.',
        );
    });

    it('should stay readable when parameters are missing or malformed', () => {
        expect(errorMessage(en, new ApiError(403, 'forbidden', { permission: 42 }))).toBe(
            'Your account does not hold the permission this needs.',
        );
        expect(errorMessage(en, new ApiError(503, 'busy'))).toBe(
            'Observer is busy checking other passwords. Try again in 0 s.',
        );
    });

    it('should fall back to a generic message for a code it does not know', () => {
        expect(errorMessage(en, new ApiError(500, 'http_500'))).toBe(en.failure.unknown);
        expect(errorMessage(en, new ApiError(418, 'toString'))).toBe(en.failure.unknown);
        expect(errorMessage(en, new Error('boom'))).toBe(en.failure.unknown);
    });

    it('should say Observer cannot be reached when the request got no answer', () => {
        expect(errorMessage(en, new TypeError('Failed to fetch'))).toBe(en.failure.network);
    });

    it('should say Observer cannot be reached when a proxy in front of it answers in its place', () => {
        for (const status of [502, 503, 504]) {
            expect(errorMessage(en, new ApiError(status, `http_${status}`))).toBe(en.failure.network);
        }
        expect(errorMessage(en, new ApiError(503, 'busy'))).not.toBe(en.failure.network);
    });
});

describe('problemMessage', () => {
    it('should word a problem found in the form like the API refusing the same thing', () => {
        expect(problemMessage(en, { code: 'invalid_username' })).toBe(en.errors.invalid_username);
        expect(problemMessage(en, { code: 'password_too_long', params: { max: 1024 } })).toBe(
            'This password is too long: 1,024 bytes at most.',
        );
    });

    it('should word the problems only a form has', () => {
        expect(problemMessage(en, { code: 'required' })).toBe('Fill this in.');
        expect(problemMessage(en, { code: 'passwords_differ' })).toBe('The two passwords differ.');
    });
});
