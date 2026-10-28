import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { StoredLaunch } from '../api/generated/StoredLaunch';
import { DEFAULT_LAUNCH, launchQuery } from '../api/launch';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { ServerPage } from './ServerPage';

const stored = (cfgFile: string, version: number): StoredLaunch => ({
    settings: {
        ...DEFAULT_LAUNCH,
        buildPath: 'C:\\FXServer\\build',
        dataPath: 'C:\\FXServer\\server-data',
        cfgFile,
    },
    version,
});

/** The page over settings already read, which only the test changes. */
const page = (initial: StoredLaunch | null) => {
    const client = new QueryClient({ defaultOptions: { queries: { staleTime: Infinity, retry: false } } });
    client.setQueryData(launchQuery.queryKey, initial);
    const view = render(
        <QueryClientProvider client={client}>
            <I18nProvider>
                <ServerPage />
            </I18nProvider>
        </QueryClientProvider>,
    );
    const input = (label: string) => view.getByLabelText(label) as HTMLInputElement;
    // The query tells its observers on a later tick.
    const storedElsewhere = (next: StoredLaunch) =>
        act(async () => {
            client.setQueryData(launchQuery.queryKey, next);
            await new Promise((resolve) => setTimeout(resolve, 0));
        });
    return { ...view, input, storedElsewhere };
};

describe('ServerPage', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should show what another session stored while nothing was typed', async () => {
        const { input, storedElsewhere, queryByText } = page(stored('server.cfg', 1));

        await storedElsewhere(stored('configs/live.cfg', 2));

        expect(input(en.server.cfgFile).value).toBe('configs/live.cfg');
        expect(queryByText(en.server.changes(1))).toBeNull();
        expect(queryByText(en.server.changedElsewhere)).toBeNull();
    });

    it('should keep what was typed, say the settings changed, and show theirs on discard', async () => {
        const { input, storedElsewhere, getByText } = page(stored('server.cfg', 1));
        fireEvent.change(input(en.server.dataPath), { target: { value: 'D:\\server-data' } });

        await storedElsewhere(stored('configs/live.cfg', 2));

        expect(input(en.server.dataPath).value).toBe('D:\\server-data');
        expect(input(en.server.cfgFile).value).toBe('server.cfg');
        getByText(en.server.changedElsewhere);

        fireEvent.click(getByText(en.server.discard));
        expect(input(en.server.dataPath).value).toBe('C:\\FXServer\\server-data');
        expect(input(en.server.cfgFile).value).toBe('configs/live.cfg');
    });

    it('should save naming the version the draft was made from', async () => {
        const fetch = vi.fn(async () => Response.json(stored('configs/live.cfg', 2)));
        vi.stubGlobal('fetch', fetch);
        const { input, getByText } = page(stored('server.cfg', 1));
        fireEvent.change(input(en.server.cfgFile), { target: { value: 'configs/live.cfg' } });

        fireEvent.click(getByText(en.server.save));

        await waitFor(() => getByText(en.server.saved));
        const [, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
        expect(JSON.parse(String(init.body))).toEqual({
            settings: stored('configs/live.cfg', 1).settings,
            version: 1,
        });
    });
});
