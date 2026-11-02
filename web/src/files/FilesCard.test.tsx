import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { MyFileAccess } from '../api/generated/MyFileAccess';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { FilesCard } from './FilesCard';

const MINE: MyFileAccess = { login: 'ada', groups: [], keys: [] };

/** The card, `GET /api/account/files` answering `status` with `body`. */
const card = (status: number, body: unknown) => {
    vi.stubGlobal(
        'fetch',
        vi.fn(
            async () =>
                new Response(JSON.stringify(body), {
                    status,
                    headers: { 'content-type': 'application/json' },
                }),
        ),
    );
    return render(
        <QueryClientProvider client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}>
            <I18nProvider>
                <FilesCard />
            </I18nProvider>
        </QueryClientProvider>,
    );
};

describe('FilesCard', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it("should show the account's login while SFTP runs", async () => {
        const { findByText } = card(200, MINE);

        expect(await findByText(en.myAccount.files.login)).not.toBeNull();
    });

    it('should stay away while SFTP does not run', async () => {
        const off = { code: 'feature_disabled', params: { feature: 'sftp', held: 'notConfigured' } };

        const { container } = card(404, off);

        await waitFor(() => expect(container.innerHTML).toBe(''));
    });
});
