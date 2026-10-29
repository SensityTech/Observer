import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';

import { aboutQuery, licencesQuery } from '../api/about';
import { ApiError } from '../api/client';
import type { About } from '../api/generated/About';
import type { ThirdPartyLicences } from '../api/generated/ThirdPartyLicences';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { AboutPage } from './AboutPage';

const release: About = {
    version: '0.1.0',
    commit: '3f2a9c1b7d4e',
    commitDate: '2026-10-28',
    os: 'windows',
    arch: 'x86_64',
    profile: 'release',
};

const licences: ThirdPartyLicences = {
    target: 'x86_64-pc-windows-msvc',
    packages: [
        {
            ecosystem: 'crate',
            name: 'serde',
            version: '1.0.228',
            licence: 'MIT OR Apache-2.0',
            repository: 'https://github.com/serde-rs/serde',
            files: [{ path: 'LICENSE-MIT', text: 'mit' }],
            fromSpdx: false,
        },
        {
            ecosystem: 'crate',
            name: 'ts-rs',
            version: '12.0.1',
            licence: 'MIT',
            repository: null,
            files: [{ path: 'MIT (SPDX)', text: 'mit' }],
            fromSpdx: true,
        },
        {
            ecosystem: 'npm',
            name: 'lightweight-charts',
            version: '5.2.1',
            licence: 'Apache-2.0',
            repository: 'git+https://github.com/tradingview/lightweight-charts.git',
            files: [
                { path: 'LICENSE', text: 'apache' },
                { path: 'NOTICE', text: 'notice' },
            ],
            fromSpdx: false,
        },
    ],
    texts: {
        mit: 'Permission is hereby granted, free of charge',
        apache: 'Apache License Version 2.0',
        notice: 'TradingView Lightweight Charts, copyright TradingView, Inc.',
    },
};

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

const page = async (about: About, list: ThirdPartyLicences | 'missing') => {
    const client = new QueryClient({
        defaultOptions: { queries: { staleTime: Infinity, retry: false, retryOnMount: false } },
    });
    client.setQueryData(aboutQuery.queryKey, about);
    if (list === 'missing') {
        await client.prefetchQuery({
            ...licencesQuery,
            queryFn: () => Promise.reject(new ApiError(404, 'not_found', {})),
        });
    } else {
        client.setQueryData(licencesQuery.queryKey, list);
    }
    const view = render(
        <QueryClientProvider client={client}>
            <I18nProvider>
                <AboutPage />
            </I18nProvider>
        </QueryClientProvider>,
    );
    await settle();
    return view;
};

describe('AboutPage', () => {
    beforeAll(() => {
        // jsdom has the element but not its modal methods.
        HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
            this.open = true;
        };
        HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
            this.open = false;
        };
    });

    afterEach(cleanup);

    it('should show the build, its commit and its platform', async () => {
        const view = await page(release, licences);

        expect(view.getByText('3f2a9c1b7d4e')).toBeTruthy();
        expect(view.getByText('Oct 28, 2026')).toBeTruthy();
        expect(view.getByText('Windows x86_64')).toBeTruthy();
        expect(view.getByText(en.about.profiles.release)).toBeTruthy();
    });

    it('should not name the compile profile with the word the status bar uses for FXServer', async () => {
        const view = await page(release, licences);

        expect(view.getByText(en.about.profile)).toBeTruthy();
        expect(view.queryByText(en.status.build)).toBeNull();
    });

    it('should say a build without a commit is a development build', async () => {
        const view = await page({ ...release, commit: null, commitDate: null, profile: 'debug' }, licences);

        expect(view.getByText(en.about.development)).toBeTruthy();
        expect(view.getByText(en.about.profiles.debug)).toBeTruthy();
    });

    it('should count the packages of each ecosystem', async () => {
        const view = await page(release, licences);

        expect(view.getByText('2 crates · 1 npm package')).toBeTruthy();
    });

    it('should find packages by name and by licence', async () => {
        const view = await page(release, licences);
        const search = view.getByRole('searchbox', { name: en.about.search });
        const names = () =>
            view
                .queryAllByRole('row')
                .map((row) => row.querySelector('td')?.textContent)
                .filter(Boolean);

        fireEvent.change(search, { target: { value: 'light' } });
        await settle();
        expect(names()).toEqual(['lightweight-charts']);

        fireEvent.change(search, { target: { value: 'apache' } });
        await settle();
        expect(names()).toEqual(['serde', 'lightweight-charts']);

        fireEvent.change(search, { target: { value: 'gpl' } });
        await settle();
        expect(view.getByText(en.about.noMatch)).toBeTruthy();
    });

    it('should open a package with its notice, and say when the text comes from SPDX', async () => {
        const view = await page(release, licences);

        fireEvent.click(view.getByRole('button', { name: en.about.textOf('lightweight-charts', '5.2.1') }));
        await settle();
        const panel = within(view.getByRole('dialog'));
        expect(panel.getByText('NOTICE')).toBeTruthy();
        expect(panel.getByText(licences.texts.notice ?? '')).toBeTruthy();
        expect(panel.queryByText(en.about.fromSpdx)).toBeNull();
        fireEvent.click(panel.getByRole('button', { name: en.common.close }));
        await settle();

        fireEvent.click(view.getByRole('button', { name: en.about.textOf('ts-rs', '12.0.1') }));
        await settle();
        expect(within(view.getByRole('dialog')).getByText(en.about.fromSpdx)).toBeTruthy();
    });

    it('should say when the build carries no licence list', async () => {
        const view = await page(release, 'missing');

        expect(view.getByText(en.about.missing)).toBeTruthy();
    });
});
