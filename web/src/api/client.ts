export class ApiError extends Error {
    constructor(
        readonly status: number,
        readonly code: string,
    ) {
        super(`API ${status}: ${code}`);
        this.name = 'ApiError';
    }
}

/** Reads a JSON resource under `/api`. Errors carry a code the panel translates, never a sentence. */
export const getJson = async <T>(path: string, signal?: AbortSignal): Promise<T> => {
    const response = await fetch(`/api${path}`, {
        headers: { accept: 'application/json' },
        credentials: 'same-origin',
        signal,
    });
    if (!response.ok) {
        throw new ApiError(response.status, await errorCode(response));
    }
    return (await response.json()) as T;
};

const errorCode = async (response: Response) => {
    try {
        const body: unknown = await response.json();
        if (typeof body === 'object' && body !== null && 'code' in body && typeof body.code === 'string') {
            return body.code;
        }
    } catch {
        // Not JSON: fall back to the status below.
    }
    return `http_${response.status}`;
};
