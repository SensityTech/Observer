import './styles/app.css';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { meQuery } from './api/auth';
import { ApiError, configureSession } from './api/client';
import { openSocket, requestTicket } from './api/live';
import { I18nProvider } from './i18n/I18nProvider';
import { LiveContext } from './live/context';
import { Realtime } from './live/realtime';
import { createPanelRouter } from './router';

const queryClient = new QueryClient({
    defaultOptions: {
        queries: {
            // A refusal stays a refusal; only a request that got no answer is worth another try.
            retry: (failures, error) => !(error instanceof ApiError) && failures < 1,
            refetchOnWindowFocus: true,
        },
    },
});

const router = createPanelRouter(queryClient);

const live = new Realtime({
    ticket: requestTicket,
    connect: openSocket,
    refetch: () => void queryClient.invalidateQueries(),
});

configureSession({
    // The routes' guards run again and send the user where they now belong.
    ended: () => {
        queryClient.removeQueries({ predicate: (query) => query.queryKey[0] !== 'me' });
        queryClient.setQueryData(meQuery.queryKey, null);
        void router.invalidate();
    },
    passwordChangeRequired: () => {
        void queryClient.invalidateQueries({ queryKey: meQuery.queryKey }).then(() => router.invalidate());
    },
});

const root = document.getElementById('root');
if (root === null) {
    throw new Error('index.html has no #root element');
}

createRoot(root).render(
    <StrictMode>
        <I18nProvider>
            <QueryClientProvider client={queryClient}>
                <LiveContext value={live}>
                    <RouterProvider router={router} />
                </LiveContext>
            </QueryClientProvider>
        </I18nProvider>
    </StrictMode>,
);
