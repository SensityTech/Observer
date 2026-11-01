import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { StoredLaunch } from '../api/generated/StoredLaunch';
import { DEFAULT_LAUNCH, launchQuery } from '../api/launch';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { ServerPage } from './ServerPage';

// The page links to the Artifacts page; no router runs here.
vi.mock('@tanstack/react-router', () => ({
    Link: ({ to, children, className }: { to: string; children: ReactNode; className?: string }) => (
        <a href={to} className={className}>
            {children}
        </a>
    ),
}));

const stored = (cfgFile: string, version: number): StoredLaunch => ({
    settings: {
        ...DEFAULT_LAUNCH,
        dataPath: 'C:\\FXServer\\server-data',
        cfgFile,
        extraArguments: [
            { name: 'sv_hostname', value: 'Lab' },
            { name: 'sv_maxclients', value: '48' },
        ],
    },
    version,
    legacyBuildPath: null,
});

/** Settings stored with a hang delay of `seconds`. */
const delayed = (seconds: number, version = 1, cfgFile = 'server.cfg'): StoredLaunch => {
    const launch = stored(cfgFile, version);
    return { ...launch, settings: { ...launch.settings, hangTimeoutSeconds: seconds } };
};

const follows = (first: Node, second: Node) =>
    Boolean(first.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING);

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

    it('should leave the build to the Artifacts page and show the folder FXServer ran from before', () => {
        const legacy = { ...stored('server.cfg', 1), legacyBuildPath: '/srv/fxserver/build' };

        const { getByText, queryByLabelText } = page(legacy);

        expect(queryByLabelText(/build folder/i)).toBeNull();
        expect(getByText(en.server.legacyBuild)).toBeTruthy();
        expect(getByText('/srv/fxserver/build').className).toContain('font-mono');
        expect(getByText(en.server.openArtifacts).closest('a')?.getAttribute('href')).toBe('/artifacts');
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

    it('should keep the keyboard on the arguments as they are removed', () => {
        const { getByLabelText, getByText, getAllByLabelText } = page(stored('server.cfg', 1));

        const first = getByLabelText(en.server.removeArgument(1));
        first.focus();
        fireEvent.click(first);
        expect(document.activeElement).toBe(getAllByLabelText(en.server.argumentName)[0]);
        expect((document.activeElement as HTMLInputElement).value).toBe('sv_maxclients');

        fireEvent.click(getByLabelText(en.server.removeArgument(1)));
        expect(document.activeElement).toBe(getByText(en.server.addArgument).closest('button'));
    });

    it('should move the keyboard to what happened once its button goes', async () => {
        const fetch = vi.fn(async () => Response.json(stored('configs/live.cfg', 2)));
        vi.stubGlobal('fetch', fetch);
        const { input, getByText } = page(stored('server.cfg', 1));

        fireEvent.change(input(en.server.cfgFile), { target: { value: 'configs/other.cfg' } });
        getByText(en.server.discard).focus();
        fireEvent.click(getByText(en.server.discard));
        expect(document.activeElement).toBe(getByText(en.server.discarded).closest('[role]'));

        fireEvent.change(input(en.server.cfgFile), { target: { value: 'configs/live.cfg' } });
        getByText(en.server.save).focus();
        fireEvent.click(getByText(en.server.save));
        await waitFor(() => getByText(en.server.saved));
        expect(document.activeElement).toBe(getByText(en.server.saved).closest('[role]'));
    });

    it('should leave the keyboard in an argument saved with Enter', async () => {
        const fetch = vi.fn(async () => Response.json(stored('server.cfg', 2)));
        vi.stubGlobal('fetch', fetch);
        const { getAllByLabelText, getByText } = page(stored('server.cfg', 1));
        const value = getAllByLabelText(en.server.argumentValue)[0] as HTMLInputElement;

        fireEvent.change(value, { target: { value: 'Lab 2' } });
        value.focus();
        fireEvent.submit(value.form as HTMLFormElement);
        await waitFor(() => getByText(en.server.saved));

        expect(document.activeElement).toBe(value);
    });

    it('should save hang detection with the other settings, its delay typed in minutes and sent in seconds', async () => {
        const fetch = vi.fn(async () => Response.json(stored('server.cfg', 2)));
        vi.stubGlobal('fetch', fetch);
        const { input, getByText } = page(stored('server.cfg', 1));
        const detection = input(en.server.hangDetection);
        expect(detection.checked).toBe(true);
        expect(input(en.server.hangTimeout).value).toBe('6');

        fireEvent.click(detection);
        fireEvent.change(input(en.server.hangTimeout), { target: { value: '15' } });
        getByText(en.server.changes(2));
        fireEvent.click(getByText(en.server.save));

        await waitFor(() => getByText(en.server.savedHang));
        const [, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
        expect(JSON.parse(String(init.body))).toEqual({
            settings: { ...stored('server.cfg', 1).settings, hangDetection: false, hangTimeoutSeconds: 900 },
            version: 1,
        });
    });

    it('should say hang detection applies at once and the rest at the next start when a save changes both', async () => {
        vi.stubGlobal(
            'fetch',
            vi.fn(async () => Response.json(stored('configs/live.cfg', 2))),
        );
        const { input, getByText, queryByText } = page(stored('server.cfg', 1));
        getByText(en.server.nextStart);

        fireEvent.change(input(en.server.cfgFile), { target: { value: 'configs/live.cfg' } });
        fireEvent.change(input(en.server.hangTimeout), { target: { value: '15' } });
        fireEvent.click(getByText(en.server.save));

        await waitFor(() => getByText(en.server.savedBoth));
        expect(queryByText(en.server.saved)).toBeNull();
    });

    it('should never count a stored delay the field cannot show exactly as an unsaved change', async () => {
        const { input, getByText, queryByText, storedElsewhere } = page(delayed(500));

        expect(input(en.server.hangTimeout).value).toBe('8.33');
        expect(queryByText(en.server.changes(1))).toBeNull();

        // With nothing typed, what another session stores still replaces what the form shows.
        await storedElsewhere(delayed(500, 2, 'configs/live.cfg'));
        expect(input(en.server.cfgFile).value).toBe('configs/live.cfg');
        expect(queryByText(en.server.changedElsewhere)).toBeNull();

        fireEvent.click(input(en.server.onesyncModes.legacy));
        getByText(en.server.changes(1));
        fireEvent.click(getByText(en.server.discard));
        expect(queryByText(en.server.changes(1))).toBeNull();
    });

    it('should take the stored delay written another way as no change', () => {
        const { input, queryByText } = page(stored('server.cfg', 1));

        for (const typed of ['6.0', '06', '6.000']) {
            fireEvent.change(input(en.server.hangTimeout), { target: { value: typed } });
            expect(queryByText(en.server.changes(1)), typed).toBeNull();
        }
        fireEvent.change(input(en.server.hangTimeout), { target: { value: '7' } });
        expect(queryByText(en.server.changes(1))).not.toBeNull();
    });

    it('should ask for whole minutes over a stored delay that is not, as Observer does, then send the one typed', async () => {
        const fetch = vi.fn(async () => Response.json(stored('server.cfg', 2)));
        vi.stubGlobal('fetch', fetch);
        Element.prototype.scrollIntoView = vi.fn();
        const { input, getByText } = page(delayed(90));
        const delay = input(en.server.hangTimeout);
        expect(delay.value).toBe('1.5');

        fireEvent.click(input(en.server.onesyncModes.legacy));
        fireEvent.click(getByText(en.server.save));
        getByText(en.server.problems.outOfRange);
        expect(document.activeElement).toBe(delay);
        expect(fetch).not.toHaveBeenCalled();

        fireEvent.change(delay, { target: { value: '2' } });
        fireEvent.click(getByText(en.server.save));
        await waitFor(() => getByText(en.server.savedBoth));
        const [, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
        expect(JSON.parse(String(init.body))).toEqual({
            settings: { ...delayed(90).settings, onesync: 'legacy', hangTimeoutSeconds: 120 },
            version: 1,
        });
    });

    it('should refuse a delay that is not 1 to 60 whole minutes next to its field, without sending it', () => {
        const fetch = vi.fn();
        vi.stubGlobal('fetch', fetch);
        Element.prototype.scrollIntoView = vi.fn();
        const { input, getByText, queryByText } = page(stored('server.cfg', 1));
        const delay = input(en.server.hangTimeout);

        for (const typed of ['0', '61', '1.5', '']) {
            fireEvent.change(delay, { target: { value: typed } });
            expect(queryByText(en.server.problems.outOfRange)).toBeNull();
            fireEvent.click(getByText(en.server.save));

            getByText(en.server.problems.outOfRange);
            expect(delay.getAttribute('aria-invalid')).toBe('true');
            expect(document.activeElement).toBe(delay);
        }
        expect(fetch).not.toHaveBeenCalled();
        fireEvent.change(delay, { target: { value: '60' } });
        expect(queryByText(en.server.problems.outOfRange)).toBeNull();
    });

    it('should put each refused argument message under its own field, and the keyboard on the first', async () => {
        const refusal = {
            code: 'invalid_launch_settings',
            params: { issues: [{ field: 'argumentName', index: 1, problem: 'duplicate' }] },
        };
        vi.stubGlobal(
            'fetch',
            vi.fn(async () => Response.json(refusal, { status: 400 })),
        );
        const scrolled = vi.fn();
        Element.prototype.scrollIntoView = scrolled;
        const { getAllByLabelText, getByLabelText, getByText } = page(stored('server.cfg', 1));
        const name = getAllByLabelText(en.server.argumentName)[1] as HTMLInputElement;
        fireEvent.change(name, { target: { value: 'SV_HOSTNAME' } });

        fireEvent.click(getByText(en.server.save));
        const message = await waitFor(() => getByText(en.server.problems.duplicate));

        const value = getAllByLabelText(en.server.argumentValue)[1] as HTMLInputElement;
        const remove = getByLabelText(en.server.removeArgument(2));
        expect(follows(name, message)).toBe(true);
        expect(follows(message, value)).toBe(true);
        expect(follows(value, remove)).toBe(true);
        expect(document.activeElement).toBe(name);
        expect(scrolled).toHaveBeenCalledWith({ block: 'center' });
    });
});
