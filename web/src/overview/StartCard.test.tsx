import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { Me } from '../api/generated/Me';
import type { ServerStatus } from '../api/generated/ServerStatus';
import type { StartProgress } from '../api/generated/StartProgress';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { StartCard } from './StartCard';

vi.mock('@tanstack/react-router', () => ({
    Link: ({
        to,
        params,
        hash,
        children,
    }: {
        to: string;
        params?: Record<string, string>;
        hash?: string;
        children: ReactNode;
    }) => {
        const path = to.replace(/\$(\w+)/g, (_, name: string) => params?.[name] ?? '');
        return <a href={hash === undefined ? path : `${path}#${hash}`}>{children}</a>;
    },
}));

const NOW = Date.parse('2026-10-28T14:06:42Z');
const ago = (seconds: number) => new Date(NOW - seconds * 1000).toISOString();

const OWNER: Me = { id: 1, username: 'ada', isOwner: true, permissions: 'fff', mustChangePassword: false };
/** Starts and stops the server, and no more. */
const OPERATOR: Me = { ...OWNER, id: 2, username: 'kenji', isOwner: false, permissions: '9' };
const VIEWER: Me = { ...OPERATOR, id: 3, username: 'grace', permissions: '1' };

/** A start 4 min 50 s in, silent for 2 min 32 s at a category of 150 resources, stuck in 3 min 28 s. */
const START: StartProgress = {
    started: 152,
    failed: 0,
    found: 391,
    planned: 380,
    last: { name: 'core', at: ago(152), warnings: 1 },
    at: {
        file: 'server.cfg',
        line: 226,
        target: '[clothes]',
        since: ago(152),
        done: 12,
        of: 150,
        resource: null,
    },
    slowest: [
        { name: 'maps', ms: 5700 },
        { name: 'docks', ms: 4900 },
        { name: 'bank', ms: 62_000 },
    ],
    stuckAt: new Date(NOW + 208_000).toISOString(),
    extensionMinutes: 15,
};

const status = (start: StartProgress): ServerStatus => ({
    state: 'starting',
    cause: { kind: 'requested', by: 'ada' },
    since: ago(290),
    startedAt: ago(290),
    generation: 1,
    problems: [],
    build: null,
    players: null,
    restartAt: null,
    launch: 'ready',
    name: null,
    port: null,
    start,
    stopHolders: [],
    stop: null,
});

const card = (start: StartProgress = START, me: Me = OWNER) =>
    render(
        <QueryClientProvider client={new QueryClient()}>
            <I18nProvider>
                <MeContext value={me}>
                    <StartCard status={status(start)} start={start} />
                </MeContext>
            </I18nProvider>
        </QueryClientProvider>,
    );

const words = en.overview.start;

describe('StartCard', () => {
    beforeEach(() => {
        vi.useFakeTimers({ toFake: ['Date'] });
        vi.setSystemTime(NOW);
    });

    afterEach(() => {
        cleanup();
        vi.useRealTimers();
        vi.unstubAllGlobals();
    });

    it('should say how far the start is, the command of the cfg it is at and the last resource up', () => {
        const { getByRole, getByText } = card();

        expect(getByRole('heading', { level: 2 }).textContent).toBe('152 resources started');
        expect(getByText('of the 380 the cfg starts')).not.toBeNull();
        const bar = getByRole('progressbar', { name: words.progress });
        expect([bar.getAttribute('value'), bar.getAttribute('aria-valuetext')]).toEqual(['40', '152 of 380']);
        const texts = (term: string) =>
            [...(getByText(term).parentElement?.querySelectorAll('dd') ?? [])].map(
                (line) => line.textContent,
            );
        expect(texts(words.now)).toEqual([
            '[clothes]',
            'server.cfg line 226, for 2 min 32 s · 12 of its 150 are up',
            words.quiet,
        ]);
        expect(texts(words.last)).toEqual(['core', '2 min 32 s ago, with 1 warning']);
        expect(
            within(getByRole('list', { name: words.slowest }))
                .getAllByRole('listitem')
                .map((item) => item.textContent),
        ).toEqual(['maps5.7 s', 'docks4.9 s', 'bank1 min 2 s']);
    });

    it('should count against what FXServer found when the cfg could not be read whole, and say so', () => {
        const { getByText, queryByText } = card({ ...START, planned: null, at: null });

        expect(getByText('of the 391 FXServer found; the cfg decides how many it starts')).not.toBeNull();
        expect(getByText(words.unread)).not.toBeNull();
        expect(queryByText(words.quiet)).toBeNull();
    });

    it('should name the resource whose scripts load, and the resources that did not start', () => {
        const { getByText, getByRole } = card({
            ...START,
            failed: 2,
            at: { ...START.at!, target: 'framework', done: 0, of: 1, resource: 'framework' },
        });

        expect(getByText('server.cfg line 226, for 2 min 32 s')).not.toBeNull();
        expect(getByText(words.nowScripts('framework'))).not.toBeNull();
        expect(getByText('2 resources did not start.')).not.toBeNull();
        expect(getByRole('link', { name: words.openConsole }).getAttribute('href')).toBe('/console');
    });

    it('should give a silent start more time, for whoever controls the server', async () => {
        const fetch = vi.fn(async () => new Response(null, { status: 204 }));
        vi.stubGlobal('fetch', fetch);
        const { getByRole, getByText } = card(START, OPERATOR);

        expect(getByText(words.silent('2 min 32 s', '3 min 28 s'))).not.toBeNull();
        const silence = getByRole('progressbar', { name: words.silence });
        expect(silence.getAttribute('value')).toBe('42');
        expect(document.querySelector('a[href="/settings/launch#setting-hangDetection"]')).toBeNull();
        fireEvent.click(getByRole('button', { name: 'Give this start 15 min more' }));

        await waitFor(() => expect(fetch).toHaveBeenCalledTimes(1));
        const [url, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
        expect([url, init.method]).toEqual(['/api/servers/default/start/extend', 'POST']);
    });

    it('should show the silence to an account that cannot act on it, and its setting to the owner', () => {
        const viewer = card(START, VIEWER);
        expect(viewer.getByText(words.silent('2 min 32 s', '3 min 28 s'))).not.toBeNull();
        expect(viewer.queryByRole('button')).toBeNull();
        cleanup();

        const owner = card();
        expect(owner.getByRole('link', { name: words.hangDetection }).getAttribute('href')).toBe(
            '/settings/launch#setting-hangDetection',
        );
    });

    it('should keep quiet about a silence while resources start, or with hang detection off', () => {
        const busy = card({ ...START, last: { name: 'core', at: ago(12), warnings: 0 } });
        expect(busy.queryByRole('progressbar', { name: words.silence })).toBeNull();
        expect(busy.getByText('12 s ago')).not.toBeNull();
        cleanup();

        const unwatched = card({ ...START, stuckAt: null });
        expect(unwatched.queryByRole('progressbar', { name: words.silence })).toBeNull();
        expect(unwatched.queryByRole('button')).toBeNull();
    });

    it('should say why more time was not given', async () => {
        vi.stubGlobal(
            'fetch',
            vi.fn(async () => Response.json({ code: 'server_not_starting' }, { status: 409 })),
        );
        const { getByRole, findByRole } = card();

        fireEvent.click(getByRole('button', { name: words.extend(15) }));

        expect((await findByRole('alert')).textContent).toBe(
            words.extendFailed(en.errors.server_not_starting),
        );
    });
});
