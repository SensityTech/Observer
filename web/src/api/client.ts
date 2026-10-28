import { clearSessionKey, readSessionKey } from './session';

export class ApiError extends Error {
    constructor(
        readonly status: number,
        readonly code: string,
        readonly params: Record<string, unknown> = {},
    ) {
        super(`API ${status}: ${code}`);
        this.name = 'ApiError';
    }
}

type Method = 'GET' | 'POST' | 'PATCH' | 'DELETE';

/** What the rest of the panel does when the API says the session changed under it. */
type SessionHooks = {
    /** Asks for the password again; true once the API accepted it. */
    reauthenticate: () => Promise<boolean>;
    ended: () => void;
    passwordChangeRequired: () => void;
};

const hooks: SessionHooks = {
    reauthenticate: async () => false,
    ended: () => {},
    passwordChangeRequired: () => {},
};

export const configureSession = (next: Partial<SessionHooks>) => {
    Object.assign(hooks, next);
};

/**
 * Sends a request under `/api` with the session key. A mutation refused for want of a recent
 * password asks for it, then goes again once.
 */
export const request = async (
    method: Method,
    path: string,
    { body, signal }: { body?: unknown; signal?: AbortSignal } = {},
): Promise<Response> => {
    const send = () => {
        const headers: Record<string, string> = { accept: 'application/json' };
        const key = readSessionKey();
        if (key !== null) {
            headers['x-observer-session-key'] = key;
        }
        // The API refuses a mutation that is not JSON, body or not.
        if (method !== 'GET') {
            headers['content-type'] = 'application/json';
        }
        return fetch(`/api${path}`, {
            method,
            headers,
            body: body === undefined ? undefined : JSON.stringify(body),
            credentials: 'same-origin',
            signal,
        });
    };

    let response = await send();
    if (!response.ok) {
        let error = await apiError(response);
        if (error.code === 'reauth_required' && (await hooks.reauthenticate())) {
            response = await send();
            if (response.ok) {
                return response;
            }
            error = await apiError(response);
        }
        if (error.code === 'unauthenticated') {
            clearSessionKey();
            hooks.ended();
        } else if (error.code === 'password_change_required') {
            hooks.passwordChangeRequired();
        }
        throw error;
    }
    return response;
};

export const getJson = async <T>(path: string, signal?: AbortSignal): Promise<T> =>
    (await (await request('GET', path, { signal })).json()) as T;

/** A mutation; answers without a body (204) resolve to undefined. */
export const send = async <T = undefined>(method: Exclude<Method, 'GET'>, path: string, body?: unknown) => {
    const response = await request(method, path, { body });
    return (response.status === 204 ? undefined : await response.json()) as T;
};

const apiError = async (response: Response) => {
    try {
        const body: unknown = await response.json();
        if (typeof body === 'object' && body !== null && 'code' in body && typeof body.code === 'string') {
            const params =
                'params' in body && typeof body.params === 'object' && body.params !== null
                    ? (body.params as Record<string, unknown>)
                    : {};
            return new ApiError(response.status, body.code, params);
        }
    } catch {
        // Not JSON: fall back to the status below.
    }
    return new ApiError(response.status, `http_${response.status}`);
};
