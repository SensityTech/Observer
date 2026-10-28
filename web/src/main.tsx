import './styles/app.css';

import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { RouterProvider } from '@tanstack/react-router';
import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { I18nProvider } from './i18n/I18nProvider';
import { router } from './router';

const queryClient = new QueryClient({
    defaultOptions: {
        queries: { retry: 1, refetchOnWindowFocus: true },
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
                <RouterProvider router={router} />
            </QueryClientProvider>
        </I18nProvider>
    </StrictMode>,
);
