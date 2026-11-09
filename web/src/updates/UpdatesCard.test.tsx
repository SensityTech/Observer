import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { UpdatesView } from '../api/generated/UpdatesView';
import { updatesQuery } from '../api/updates';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { UpdatesCard } from './UpdatesCard';

vi.mock('@tanstack/react-router', async (original) => ({
    ...(await original<typeof import('@tanstack/react-router')>()),
    Link: ({ className, children }: { className?: string; children: ReactNode }) => (
        <a href="/settings/features" className={className}>
            {children}
        </a>
    ),
}));

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const RELEASE = {
    version: '0.2.0',
    name: 'Observer 0.2.0',
    publishedAt: '2026-11-09T10:00:00Z',
    notes: "## What's Changed\n* Install updates from the panel in https://github.com/SensityTech/Observer/pull/45",
    notesCut: false,
    url: 'https://github.com/SensityTech/Observer/releases/tag/v0.2.0',
    asset: { name: 'observer-0.2.0-windows-x86_64.zip', size: 14_200_000 },
    signed: true,
};

const VIEW: UpdatesView = {
    current: '0.1.0',
    checkedAt: '2026-11-09T11:00:00Z',
    checking: false,
    checkFailure: null,
    available: RELEASE,
    job: null,
    last: null,
    blocked: null,
    nextRestartAt: null,
};

const card = async (view: UpdatesView) => {
    const client = new QueryClient({
        defaultOptions: { queries: { staleTime: Infinity, retry: false, retryOnMount: false } },
    });
    client.setQueryData(updatesQuery.queryKey, view);
    const rendered = render(
        <QueryClientProvider client={client}>
            <I18nProvider>
                <UpdatesCard />
            </I18nProvider>
        </QueryClientProvider>,
    );
    await settle();
    return rendered;
};

describe('UpdatesCard', () => {
    beforeAll(() => {
        HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
            this.open = true;
        };
        HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
            this.open = false;
        };
    });

    afterEach(cleanup);

    it('shows a newer release with its notes, and asks before it installs it', async () => {
        const view = await card(VIEW);

        expect(view.getByText(en.updates.from('0.1.0', '0.2.0'))).toBeTruthy();
        expect(view.getByRole('heading', { name: "What's Changed" })).toBeTruthy();
        expect(view.getByRole('link', { name: '#45' })).toBeTruthy();
        expect(view.getByText(en.updates.whenAbout.stopped)).toBeTruthy();
        fireEvent.click(view.getByRole('button', { name: en.updates.install('0.2.0') }));

        expect(view.getByRole('dialog', { name: en.updates.confirm('0.2.0') })).toBeTruthy();
        expect(view.getByText(en.updates.rollbackNote)).toBeTruthy();
    });

    it('says why a release cannot be installed rather than offer it', async () => {
        const blocked = await card({ ...VIEW, blocked: 'readOnly' });
        expect(blocked.getByText(en.updates.blocked.readOnly)).toBeTruthy();
        expect(blocked.queryByRole('button', { name: en.updates.install('0.2.0') })).toBeNull();
        cleanup();

        const unsigned = await card({ ...VIEW, available: { ...RELEASE, signed: false } });
        expect(unsigned.getByText(en.updates.unsignedRelease)).toBeTruthy();
        expect(unsigned.queryByRole('button', { name: en.updates.install('0.2.0') })).toBeNull();
    });

    it('follows an install as it downloads', async () => {
        const view = await card({
            ...VIEW,
            job: {
                version: '0.2.0',
                when: 'now',
                step: 'download',
                state: 'running',
                startedBy: 'ada',
                startedAt: '2026-11-09T11:00:00Z',
                finishedAt: null,
                download: { received: 7_100_000, total: 14_200_000, rate: 2_000_000 },
                restartAt: null,
                failure: null,
            },
        });

        expect(view.getByRole('list', { name: en.updates.stepsLabel('0.2.0') })).toBeTruthy();
        expect(view.getByRole('progressbar', { name: en.updates.progress }).getAttribute('value')).toBe('50');
        expect(view.getByRole('button', { name: en.updates.cancel })).toBeTruthy();
        expect(view.queryByRole('button', { name: en.updates.install('0.2.0') })).toBeNull();
    });

    it('says the new release did not start and the previous one is back', async () => {
        const view = await card({
            ...VIEW,
            available: null,
            last: {
                from: '0.1.0',
                to: '0.2.0',
                at: new Date().toISOString(),
                result: { kind: 'rolledBack', reason: 'failed' },
            },
        });

        expect(
            view.getByText(
                new RegExp(en.updates.rollbackReasons.failed.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')),
            ),
        ).toBeTruthy();
        expect(view.getByText(en.updates.upToDate('0.1.0'))).toBeTruthy();
    });

    it('tells a failed check apart from a release that is not there', async () => {
        const view = await card({ ...VIEW, available: null, checkFailure: { code: 'noRelease' } });

        expect(view.getByText(en.updates.checkFailures.noRelease)).toBeTruthy();
    });
});
