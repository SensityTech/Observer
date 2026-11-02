import { QueryClient } from '@tanstack/react-query';
import { act, cleanup, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { StoredLaunch } from '../api/generated/StoredLaunch';
import { DEFAULT_LAUNCH, launchQuery } from '../api/launch';
import { en } from '../i18n/en';
import { openSettings } from './fixtures';

const items = en.settings.items;
const bar = en.settings.bar;

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

const input = (label: string) => screen.getByLabelText(label) as HTMLInputElement;
const legacy = () => screen.getByRole('radio', { name: en.settings.launch.onesync.legacy.name });
/** Unfolds the extra arguments. */
const advanced = () =>
    fireEvent.click(screen.getByRole('button', { name: new RegExp(en.settings.launch.advanced) }));

/** The Launch category over settings already read, which only the test changes. */
const page = async (initial: StoredLaunch | null, path = '/settings/launch') => {
    const client = new QueryClient({
        defaultOptions: { queries: { staleTime: Infinity, retry: false }, mutations: { retry: false } },
    });
    client.setQueryData(launchQuery.queryKey, initial);
    const view = await openSettings(path, { client });
    // The query tells its observers on a later tick.
    const storedElsewhere = (next: StoredLaunch) =>
        act(async () => {
            client.setQueryData(launchQuery.queryKey, next);
            await new Promise((resolve) => setTimeout(resolve, 0));
        });
    return { ...view, storedElsewhere };
};

describe('Launch settings', () => {
    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should leave the build to the Artifacts page and show the folder FXServer ran from before', async () => {
        const folder = { ...stored('server.cfg', 1), legacyBuildPath: '/srv/fxserver/build' };

        const { getByText, queryByLabelText } = await page(folder);

        expect(queryByLabelText(/build folder/i)).toBeNull();
        expect(getByText(en.server.legacyBuild)).toBeTruthy();
        expect(getByText('/srv/fxserver/build').className).toContain('font-mono');
        expect(getByText(en.settings.launch.openArtifacts).closest('a')?.getAttribute('href')).toBe(
            '/artifacts',
        );
    });

    it('should explain each OneSync mode beside its choice, and say when each setting applies', async () => {
        const { getByText } = await page(stored('server.cfg', 1));

        for (const mode of ['on', 'legacy', 'off'] as const) {
            const choice = screen.getByRole('radio', { name: en.settings.launch.onesync[mode].name });
            expect(document.getElementById(choice.getAttribute('aria-describedby') ?? '')?.textContent).toBe(
                en.settings.launch.onesync[mode].about,
            );
        }
        const onesync = getByText(items.onesync.label).closest('[id^="setting-"]');
        const hang = getByText(items.hangDetection.label).closest('[id^="setting-"]');
        expect(onesync?.textContent).toContain(en.settings.applies.nextStart);
        expect(hang?.textContent).toContain(en.settings.applies.now);
    });

    it('should show what another session stored while nothing was typed', async () => {
        const { storedElsewhere, queryByText } = await page(stored('server.cfg', 1));

        await storedElsewhere(stored('configs/live.cfg', 2));

        expect(input(items.cfgFile.label).value).toBe('configs/live.cfg');
        expect(queryByText(bar.changes(1))).toBeNull();
        expect(queryByText(en.server.changedElsewhere)).toBeNull();
    });

    it('should keep what was typed, say the settings changed, and show theirs on discard', async () => {
        const { storedElsewhere, getByText } = await page(stored('server.cfg', 1));
        fireEvent.change(input(items.dataPath.label), { target: { value: 'D:\\server-data' } });

        await storedElsewhere(stored('configs/live.cfg', 2));

        expect(input(items.dataPath.label).value).toBe('D:\\server-data');
        expect(input(items.cfgFile.label).value).toBe('server.cfg');
        getByText(en.server.changedElsewhere);

        fireEvent.click(getByText(bar.discard));
        expect(input(items.dataPath.label).value).toBe('C:\\FXServer\\server-data');
        expect(input(items.cfgFile.label).value).toBe('configs/live.cfg');
    });

    it('should save naming the version the draft was made from', async () => {
        const fetch = vi.fn(async () => Response.json(stored('configs/live.cfg', 2)));
        vi.stubGlobal('fetch', fetch);
        const { getByText } = await page(stored('server.cfg', 1));
        fireEvent.change(input(items.cfgFile.label), { target: { value: 'configs/live.cfg' } });

        fireEvent.click(getByText(bar.save));

        await waitFor(() => getByText(en.server.saved));
        const [, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
        expect(JSON.parse(String(init.body))).toEqual({
            settings: stored('configs/live.cfg', 1).settings,
            version: 1,
        });
    });

    it('should keep the extra arguments folded until asked for, saying how many are set', async () => {
        const { getByRole, getAllByLabelText } = await page(stored('server.cfg', 1));
        const disclosure = getByRole('button', { name: new RegExp(en.settings.launch.advanced) });

        expect(disclosure.textContent).toContain(en.settings.launch.argumentsSet(2));
        expect(disclosure.getAttribute('aria-expanded')).toBe('false');
        expect(getAllByLabelText(en.server.argumentName)[0]?.closest('[hidden]')).not.toBeNull();
        fireEvent.click(disclosure);
        expect(getAllByLabelText(en.server.argumentName)[0]?.closest('[hidden]')).toBeNull();
    });

    it('should open the extra arguments a search leads to', async () => {
        Element.prototype.scrollIntoView = vi.fn();
        const { getAllByLabelText } = await page(
            stored('server.cfg', 1),
            '/settings/launch#setting-extraArguments',
        );

        expect(getAllByLabelText(en.server.argumentName)[0]?.closest('[hidden]')).toBeNull();
        expect(Element.prototype.scrollIntoView).toHaveBeenCalledWith({ block: 'center' });
    });

    it('should keep the keyboard on the arguments as they are removed', async () => {
        const { getByLabelText, getByText, getAllByLabelText } = await page(stored('server.cfg', 1));
        advanced();

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
        const { getByText } = await page(stored('server.cfg', 1));

        fireEvent.change(input(items.cfgFile.label), { target: { value: 'configs/other.cfg' } });
        getByText(bar.discard).focus();
        fireEvent.click(getByText(bar.discard));
        expect(document.activeElement).toBe(getByText(en.server.discarded).closest('[role]'));

        fireEvent.change(input(items.cfgFile.label), { target: { value: 'configs/live.cfg' } });
        getByText(bar.save).focus();
        fireEvent.click(getByText(bar.save));
        await waitFor(() => getByText(en.server.saved));
        expect(document.activeElement).toBe(getByText(en.server.saved).closest('[role]'));
    });

    it('should leave the keyboard in an argument saved with Enter', async () => {
        const fetch = vi.fn(async () => Response.json(stored('server.cfg', 2)));
        vi.stubGlobal('fetch', fetch);
        const { getAllByLabelText, getByText } = await page(stored('server.cfg', 1));
        advanced();
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
        const { getByText } = await page(stored('server.cfg', 1));
        const detection = input(items.hangDetection.label);
        expect(detection.checked).toBe(true);
        expect(input(items.hangTimeout.label).value).toBe('6');

        fireEvent.click(detection);
        fireEvent.change(input(items.hangTimeout.label), { target: { value: '15' } });
        getByText(bar.changes(2));
        getByText(`· ${bar.applies([en.settings.appliesWhen.now])}`);
        fireEvent.click(getByText(bar.save));

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
        const { getByText, queryByText } = await page(stored('server.cfg', 1));

        fireEvent.change(input(items.cfgFile.label), { target: { value: 'configs/live.cfg' } });
        fireEvent.change(input(items.hangTimeout.label), { target: { value: '15' } });
        getByText(`· ${bar.applies([en.settings.appliesWhen.now, en.settings.appliesWhen.nextStart])}`);
        fireEvent.click(getByText(bar.save));

        await waitFor(() => getByText(en.server.savedBoth));
        expect(queryByText(en.server.saved)).toBeNull();
    });

    it('should never count a stored delay the field cannot show exactly as an unsaved change', async () => {
        const { getByText, queryByText, storedElsewhere } = await page(delayed(500));

        expect(input(items.hangTimeout.label).value).toBe('8.33');
        expect(queryByText(bar.changes(1))).toBeNull();

        // With nothing typed, what another session stores still replaces what the form shows.
        await storedElsewhere(delayed(500, 2, 'configs/live.cfg'));
        expect(input(items.cfgFile.label).value).toBe('configs/live.cfg');
        expect(queryByText(en.server.changedElsewhere)).toBeNull();

        fireEvent.click(legacy());
        getByText(bar.changes(1));
        fireEvent.click(getByText(bar.discard));
        expect(queryByText(bar.changes(1))).toBeNull();
    });

    it('should take the stored delay written another way as no change', async () => {
        const { queryByText } = await page(stored('server.cfg', 1));

        for (const typed of ['6.0', '06', '6.000']) {
            fireEvent.change(input(items.hangTimeout.label), { target: { value: typed } });
            expect(queryByText(bar.changes(1)), typed).toBeNull();
        }
        fireEvent.change(input(items.hangTimeout.label), { target: { value: '7' } });
        expect(queryByText(bar.changes(1))).not.toBeNull();
    });

    it('should ask for whole minutes over a stored delay that is not, as Observer does, then send the one typed', async () => {
        const fetch = vi.fn(async () => Response.json(stored('server.cfg', 2)));
        vi.stubGlobal('fetch', fetch);
        Element.prototype.scrollIntoView = vi.fn();
        const { getByText } = await page(delayed(90));
        const delay = input(items.hangTimeout.label);
        expect(delay.value).toBe('1.5');

        fireEvent.click(legacy());
        fireEvent.click(getByText(bar.save));
        getByText(en.server.problems.outOfRange);
        expect(document.activeElement).toBe(delay);
        expect(fetch).not.toHaveBeenCalled();

        fireEvent.change(delay, { target: { value: '2' } });
        fireEvent.click(getByText(bar.save));
        await waitFor(() => getByText(en.server.savedBoth));
        const [, init] = fetch.mock.calls[0] as unknown as [string, RequestInit];
        expect(JSON.parse(String(init.body))).toEqual({
            settings: { ...delayed(90).settings, onesync: 'legacy', hangTimeoutSeconds: 120 },
            version: 1,
        });
    });

    it('should refuse a delay that is not 1 to 60 whole minutes next to its field, without sending it', async () => {
        const fetch = vi.fn();
        vi.stubGlobal('fetch', fetch);
        Element.prototype.scrollIntoView = vi.fn();
        const { getByText, queryByText } = await page(stored('server.cfg', 1));
        const delay = input(items.hangTimeout.label);

        for (const typed of ['0', '61', '1.5', '']) {
            fireEvent.change(delay, { target: { value: typed } });
            expect(queryByText(en.server.problems.outOfRange)).toBeNull();
            fireEvent.click(getByText(bar.save));

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
        const { getAllByLabelText, getByLabelText, getByText } = await page(stored('server.cfg', 1));
        advanced();
        const name = getAllByLabelText(en.server.argumentName)[1] as HTMLInputElement;
        fireEvent.change(name, { target: { value: 'SV_HOSTNAME' } });

        fireEvent.click(getByText(bar.save));
        const message = await waitFor(() => getByText(en.server.problems.duplicate));

        const value = getAllByLabelText(en.server.argumentValue)[1] as HTMLInputElement;
        const remove = getByLabelText(en.server.removeArgument(2));
        expect(follows(name, message)).toBe(true);
        expect(follows(message, value)).toBe(true);
        expect(follows(value, remove)).toBe(true);
        expect(document.activeElement).toBe(name);
        expect(scrolled).toHaveBeenCalledWith({ block: 'center' });
    });

    it('should open the arguments a refusal names, folded or not', async () => {
        const refusal = {
            code: 'invalid_launch_settings',
            params: { issues: [{ field: 'argumentName', index: 0, problem: 'reserved' }] },
        };
        vi.stubGlobal(
            'fetch',
            vi.fn(async () => Response.json(refusal, { status: 400 })),
        );
        Element.prototype.scrollIntoView = vi.fn();
        const { getByText, getAllByLabelText } = await page(stored('server.cfg', 1));
        fireEvent.change(input(items.cfgFile.label), { target: { value: 'configs/live.cfg' } });

        fireEvent.click(getByText(bar.save));
        await waitFor(() => getByText(en.server.problems.reserved));

        expect(getAllByLabelText(en.server.argumentName)[0]?.closest('[hidden]')).toBeNull();
    });

    it('should save with nothing stored yet, with no draft to go back to', async () => {
        const fetch = vi.fn(async () => Response.json(stored('server.cfg', 1)));
        vi.stubGlobal('fetch', fetch);
        const { getByText, queryByText } = await page(null);

        getByText(bar.notSaved);
        expect(queryByText(bar.discard)).toBeNull();
        fireEvent.change(input(items.dataPath.label), { target: { value: 'C:\\FXServer\\server-data' } });
        fireEvent.click(getByText(bar.save));

        await waitFor(() => getByText(en.server.saved));
        expect(queryByText(bar.notSaved)).toBeNull();
    });
});
