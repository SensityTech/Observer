import { ApiError } from '../api/client';
import type { ErrorCode } from '../api/generated/ErrorCode';
import type { Problem } from '../auth/validation';
import type { Messages } from './en';

const isErrorCode = (t: Messages, code: string): code is ErrorCode => Object.hasOwn(t.errors, code);

const word = (t: Messages, code: string, params: Record<string, unknown>) => {
    if (isErrorCode(t, code)) {
        const message = t.errors[code];
        return typeof message === 'function' ? message(params) : message;
    }
    return null;
};

// Observer never answers these without a code: a proxy in front of it answers them in its place.
const PROXY_ANSWERS = new Set(['http_502', 'http_503', 'http_504']);

/** What to tell the user about a failed request: the API's code in their language, or a fallback. */
export const errorMessage = (t: Messages, error: unknown) => {
    if (error instanceof ApiError) {
        return (
            word(t, error.code, error.params) ??
            (PROXY_ANSWERS.has(error.code) ? t.failure.network : t.failure.unknown)
        );
    }
    // fetch rejects with a TypeError when the request never got an answer.
    if (error instanceof TypeError) {
        return t.failure.network;
    }
    return t.failure.unknown;
};

/** A form problem found before sending, worded like the API's refusal of the same thing. */
export const problemMessage = (t: Messages, problem: Problem) => {
    if (problem.code === 'required' || problem.code === 'passwords_differ') {
        return t.validation[problem.code];
    }
    return word(t, problem.code, problem.params ?? {}) ?? t.failure.unknown;
};
