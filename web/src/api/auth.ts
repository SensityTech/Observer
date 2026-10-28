import { queryOptions, type QueryClient } from '@tanstack/react-query';

import { ApiError, getJson, send } from './client';
import type { LoginResponse } from './generated/LoginResponse';
import type { Me } from './generated/Me';
import type { SetupStatus } from './generated/SetupStatus';
import { clearSessionKey, readSessionKey, saveSessionKey } from './session';

/** The signed-in user, or null when nobody is. */
export const meQuery = queryOptions({
    queryKey: ['me'],
    queryFn: async ({ signal }): Promise<Me | null> => {
        // Without a key the cookie alone is refused: no need to ask.
        if (readSessionKey() === null) {
            return null;
        }
        try {
            return await getJson<Me>('/auth/me', signal);
        } catch (error) {
            if (error instanceof ApiError && error.status === 401) {
                return null;
            }
            throw error;
        }
    },
});

export const setupQuery = queryOptions({
    queryKey: ['setup'],
    queryFn: ({ signal }) => getJson<SetupStatus>('/setup', signal),
});

/** Keeps the session a login or the setup opened, and forgets whatever the last user saw. */
export const signIn = (queryClient: QueryClient, { sessionKey, me }: LoginResponse) => {
    saveSessionKey(sessionKey);
    queryClient.removeQueries({ predicate: (query) => query.queryKey[0] !== 'me' });
    queryClient.setQueryData(meQuery.queryKey, me);
};

export const signOut = async (queryClient: QueryClient) => {
    try {
        await send('POST', '/auth/logout');
    } finally {
        clearSessionKey();
        queryClient.removeQueries({ predicate: (query) => query.queryKey[0] !== 'me' });
        queryClient.setQueryData(meQuery.queryKey, null);
    }
};
