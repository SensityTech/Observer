import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, beforeAll, describe, expect, it, vi } from 'vitest';

import type { DiagnosticsSnapshot } from '../api/generated/DiagnosticsSnapshot';
import { MeContext } from '../auth/me';
import { ReauthDialog } from '../auth/ReauthDialog';
import { CRASH, HANG, SHELL, SNAPSHOT } from '../diagnostics/fixtures';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { DiagnosticsPage } from './DiagnosticsPage';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** The page for the owner or someone else, the records as the topic says. */
const page = async (isOwner: boolean, snapshot: DiagnosticsSnapshot = SNAPSHOT) => {
    let socket: SocketEvents | undefined;
    const live = new Realtime({
        ticket: async () => 'ticket',
        connect: (_, events) => {
            socket = events;
            return { send: () => {}, close: () => {} };
        },
        refetch: () => {},
    });
    const view = render(
        <I18nProvider>
            <MeContext
                value={{
                    id: isOwner ? 1 : 2,
                    username: isOwner ? 'ada' : 'kenji',
                    isOwner,
                    permissions: isOwner ? '1fff' : '1000',
                    mustChangePassword: false,
                }}
            >
                <LiveContext value={live}>
                    <DiagnosticsPage />
                    <ReauthDialog />
                </LiveContext>
            </MeContext>
        </I18nProvider>,
    );
    live.start();
    await settle();
    const say = (message: unknown) => act(() => socket?.message(JSON.stringify(message)));
    say({ type: 'hello', connectionId: 1, contract: 1 });
    say({ type: 'snapshot', topic: 'server.diagnostics', data: snapshot });
    return { ...view, live, say };
};

const crashes = () => {
    const section = document.querySelector<HTMLElement>('section[aria-labelledby="diagnostics-crashes"]');
    if (section === null) {
        throw new Error('no crashes card');
    }
    return within(section);
};

const answer = (status: number, body?: unknown) =>
    new Response(body === undefined ? null : JSON.stringify(body), {
        status,
        headers: { 'content-type': 'application/json' },
    });

/** Heights as the browser lays them out, which jsdom does not: the line of a `ping` clamped to two
 * lines is taller than them, everything else fits. */
const height = (clamped: number, fits: number) =>
    function (this: HTMLElement) {
        return this.classList.contains('line-clamp-2') && this.textContent?.includes('ping') ? clamped : fits;
    };

/** A crash no dump explains, as the server records it on either system. */
const ended = (id: number, exitCode: string | null, signal: number | null) => ({
    ...CRASH,
    id,
    exit: 'what the server side wrote',
    exitCode,
    signal,
    analysisState: 'noDump' as const,
    analysis: null,
    files: [],
    group: null,
});

describe('DiagnosticsPage', () => {
    beforeAll(() => {
        // jsdom has the element but not its modal methods.
        HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) {
            this.open = true;
        };
        HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) {
            this.open = false;
        };
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
    });

    it('should give the owner alone the downloads and the deletion, and keep a full dump on the host', async () => {
        const owner = await page(true);

        expect(
            owner.getByRole('button', { name: en.diagnostics.crashes.download('6c1f0b2e.dmp') }),
        ).not.toBeNull();
        expect(
            owner.queryByRole('button', { name: en.diagnostics.crashes.download('6c1f0b2e-full.dmp') }),
        ).toBeNull();
        expect(
            owner.getByText(
                `513 MB ${en.diagnostics.crashes.fileKinds.fullDump} · ${en.diagnostics.crashes.staysOnHost}`,
            ),
        ).not.toBeNull();
        expect(crashes().getAllByRole('button', { name: en.diagnostics.crashes.deleteFiles })).toHaveLength(
            2,
        );
        owner.live.stop();
        cleanup();

        const other = await page(false);

        expect(other.queryAllByRole('button', { name: /^Download / })).toHaveLength(0);
        expect(other.queryByRole('button', { name: en.diagnostics.crashes.deleteFiles })).toBeNull();
        expect(document.body.textContent).toContain('Access violation in citizen-scripting-lua.dll');
        other.live.stop();
    });

    it('should read each record in words, with its group and what its analysis found', async () => {
        const { getByText, getAllByText, getAllByRole, live } = await page(true);

        expect(getByText('svMain was waiting in')).not.toBeNull();
        expect(getByText('3 in 7 days')).not.toBeNull();
        expect(getAllByText('thread UV loop: svMain · build 35245')).toHaveLength(2);
        fireEvent.click(getAllByRole('button', { name: en.diagnostics.crashes.details })[1]!);
        expect(getByText(en.diagnostics.crashes.frames)).not.toBeNull();
        expect(getByText('citizen-scripting-lua.dll+0x1a2b3')).not.toBeNull();
        expect(getByText('?+0x7ff6a1b2c3d4')).not.toBeNull();
        live.stop();
    });

    it('should ask for the password again before deleting, then show the files removed', async () => {
        const deleted = { ...CRASH, files: CRASH.files.map((file) => ({ ...file, kept: false })) };
        const fetched: string[] = [];
        const replies = [
            answer(403, { code: 'reauth_required', params: {} }),
            answer(204),
            answer(200, deleted),
        ];
        vi.stubGlobal(
            'fetch',
            vi.fn(async (url: string, init: RequestInit) => {
                fetched.push(`${init.method} ${url}`);
                return replies.shift() ?? answer(500);
            }),
        );
        const { getAllByRole, getByRole, getByLabelText, findAllByText, live } = await page(true);

        fireEvent.click(getAllByRole('button', { name: en.diagnostics.crashes.deleteFiles })[1]!);
        expect(getByRole('alertdialog').textContent).toContain(en.diagnostics.crashes.deleteBody);
        fireEvent.click(getByRole('button', { name: en.diagnostics.crashes.deleteConfirm }));
        await settle();
        fireEvent.change(getByLabelText(en.reauth.password), {
            target: { value: 'correct horse battery staple' },
        });
        fireEvent.click(getByRole('button', { name: en.reauth.submit }));
        await settle();
        await settle();

        expect(fetched).toEqual([
            'DELETE /api/servers/default/crashes/2/files',
            'POST /api/auth/reauth',
            'DELETE /api/servers/default/crashes/2/files',
        ]);
        expect(await findAllByText(en.diagnostics.crashes.removed)).toHaveLength(2);
        live.stop();
    });

    it('should ask for the password before a download without calling it a change', async () => {
        vi.stubGlobal(
            'fetch',
            vi.fn(async () => answer(403, { code: 'reauth_required', params: {} })),
        );
        const { getByRole, live } = await page(true);

        fireEvent.click(getByRole('button', { name: en.diagnostics.crashes.download('6c1f0b2e.dmp') }));
        await settle();

        const prompt = getByRole('dialog').textContent ?? '';
        expect(prompt).toContain(en.reauth.body);
        expect(prompt).not.toMatch(/change/i);
        fireEvent.click(getByRole('button', { name: en.common.cancel }));
        await settle();
        live.stop();
    });

    it('should keep the keyboard on the page once the deleted files take their button away', async () => {
        const deleted = { ...CRASH, files: CRASH.files.map((file) => ({ ...file, kept: false })) };
        vi.stubGlobal(
            'fetch',
            vi.fn(async () => answer(200, deleted)),
        );
        const { getAllByRole, getByRole, getByText, live } = await page(true);

        const trigger = getAllByRole('button', { name: en.diagnostics.crashes.deleteFiles })[1]!;
        trigger.focus();
        fireEvent.click(trigger);
        const confirm = getByRole('button', { name: en.diagnostics.crashes.deleteConfirm });
        confirm.focus();
        fireEvent.click(confirm);
        await settle();
        await settle();

        expect(crashes().getAllByRole('button', { name: en.diagnostics.crashes.deleteFiles })).toHaveLength(
            1,
        );
        expect(document.activeElement).toBe(
            getByText(en.diagnostics.crashes.deleted).closest('[role="status"]'),
        );
        live.stop();
    });

    it('should say when a crash left no dump, and when its dump gave no frame', async () => {
        const linux = ended(9, null, 11);
        const bare = {
            ...CRASH,
            id: 10,
            analysis: { ...CRASH.analysis!, exception: null, frames: [], threads: [] },
            group: null,
        };
        const { getAllByRole, getByText, live } = await page(false, { ...SNAPSHOT, crashes: [linux, bare] });

        expect(getByText(`build 35245 · ${en.diagnostics.crashes.noDump}`)).not.toBeNull();
        fireEvent.click(getAllByRole('button', { name: en.diagnostics.crashes.details })[0]!);
        expect(getByText(en.diagnostics.crashes.noFrames)).not.toBeNull();
        live.stop();
    });

    it('should word how FXServer ended from the catalog when no dump says more', async () => {
        const { live } = await page(false, {
            ...SNAPSHOT,
            crashes: [
                ended(9, null, 11),
                ended(10, null, 77),
                ended(11, '0xC0000005', null),
                ended(12, null, null),
            ],
        });

        const cells = [
            ...document.querySelectorAll(
                'section[aria-labelledby="diagnostics-crashes"] tbody td:nth-child(2)',
            ),
        ].map((cell) => cell.textContent ?? '');
        const sentences = [
            'FXServer exited with SIGSEGV',
            `FXServer exited with ${en.alerts.signal(77)}`,
            'FXServer exited with exit code 0xC0000005',
            en.alerts.crashed,
        ];
        expect(cells).toHaveLength(4);
        cells.forEach((cell, index) => {
            expect(cell).toContain(sentences[index]);
            expect(cell).not.toContain('what the server side wrote');
        });
        live.stop();
    });

    it("should place a hang in FXServer's own module rather than in the system's wait", async () => {
        const hang = {
            ...HANG,
            analysis: {
                ...HANG.analysis!,
                frames: [
                    { module: 'ntdll.dll', offset: '0x161dd4', scanned: false },
                    { module: 'KERNELBASE.dll', offset: '0x1be9f', scanned: true },
                    { module: 'FXServer.exe', offset: '0x1034c8', scanned: true },
                ],
                module: 'FXServer.exe',
                threads: [],
                syscallsReadable: null,
            },
        };

        const { live } = await page(false, { ...SNAPSHOT, crashes: [hang] });

        const cell = document.querySelector(
            'section[aria-labelledby="diagnostics-crashes"] tbody td:nth-child(2)',
        );
        expect(cell?.textContent).toContain('svMain was stuck in FXServer.exe');
        live.stop();
    });

    it('should say what shows once FXServer runs when nothing was recorded yet', async () => {
        const empty: DiagnosticsSnapshot = {
            ...SNAPSHOT,
            processes: [],
            destinations: [],
            listeners: [],
            crashes: [],
        };

        const { getByText, live } = await page(false, empty);

        expect(getByText(en.diagnostics.crashes.emptyTitle)).not.toBeNull();
        expect(getByText(en.diagnostics.processes.emptyTitle)).not.toBeNull();
        expect(getByText(en.diagnostics.connections.none)).not.toBeNull();
        expect(getByText(en.diagnostics.connections.noListener)).not.toBeNull();
        live.stop();
    });

    it('should let a command line two lines cut short be read whole, whatever its length', async () => {
        const scroll = vi
            .spyOn(HTMLElement.prototype, 'scrollHeight', 'get')
            .mockImplementation(height(60, 20));
        const client = vi
            .spyOn(HTMLElement.prototype, 'clientHeight', 'get')
            .mockImplementation(height(40, 20));
        const line = String.raw`"cmd.exe" /c ping -n 300 -w 1000 127.0.0.1 C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe`;
        const { getAllByRole, getByRole, getByText, live } = await page(false, {
            ...SNAPSHOT,
            processes: [{ ...SHELL, commandLine: line }, SNAPSHOT.processes[1]!],
        });

        expect(line.length).toBeLessThan(120);
        expect(getAllByRole('button', { name: en.diagnostics.processes.more })).toHaveLength(1);
        fireEvent.click(getByRole('button', { name: en.diagnostics.processes.more }));
        expect(getByText(line).classList.contains('line-clamp-2')).toBe(false);
        expect(
            getByRole('button', { name: en.diagnostics.processes.less }).getAttribute('aria-expanded'),
        ).toBe('true');
        scroll.mockRestore();
        client.mockRestore();
        live.stop();
    });

    it('should mark a shell as a security alert and follow the records as they change', async () => {
        const { getByText, say, live } = await page(false);

        expect(getByText(en.diagnostics.processes.kinds.shell)).not.toBeNull();
        expect(getByText('Dumps 3 of 10 · 514 MB of 20 GB · 41 GB free')).not.toBeNull();
        say({
            type: 'event',
            topic: 'server.diagnostics',
            data: { kind: 'process', process: { ...SNAPSHOT.processes[0], endedAt: '2026-10-29T11:07:00Z' } },
        });

        expect(getByText(/^Ended \d\d:\d\d$/)).not.toBeNull();
        live.stop();
    });
});
