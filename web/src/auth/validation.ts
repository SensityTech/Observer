import type { ErrorCode } from '../api/generated/ErrorCode';

// The API's own rules, checked before sending so a mistake shows under its field at once. A problem
// carries the API's error code, so the panel words it the same way whichever side found it.

export const PASSWORD_MIN_LENGTH = 12;
export const PASSWORD_MAX_BYTES = 1024;

export type Problem = { code: ErrorCode | 'passwords_differ' | 'required'; params?: Record<string, unknown> };

const USERNAME = /^[A-Za-z0-9_.-]{3,32}$/;

export const usernameProblem = (username: string): Problem | null => {
    if (username === '') {
        return { code: 'required' };
    }
    return USERNAME.test(username) ? null : { code: 'invalid_username' };
};

export const passwordProblem = (password: string): Problem | null => {
    if (password === '') {
        return { code: 'required' };
    }
    // The API counts characters for the minimum and bytes for the maximum.
    if ([...password].length < PASSWORD_MIN_LENGTH) {
        return { code: 'password_too_short', params: { min: PASSWORD_MIN_LENGTH } };
    }
    if (new TextEncoder().encode(password).length > PASSWORD_MAX_BYTES) {
        return { code: 'password_too_long', params: { max: PASSWORD_MAX_BYTES } };
    }
    return null;
};

export const newPasswordProblem = (current: string, next: string): Problem | null =>
    passwordProblem(next) ?? (next === current ? { code: 'password_unchanged' } : null);

export const confirmationProblem = (password: string, confirmation: string): Problem | null => {
    if (confirmation === '') {
        return { code: 'required' };
    }
    return password === confirmation ? null : { code: 'passwords_differ' };
};
