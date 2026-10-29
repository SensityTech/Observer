import { act, cleanup, render } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { Alert } from '../api/generated/Alert';
import { MeContext } from '../auth/me';
import { AlertsProvider } from '../diagnostics/alerts';
import { CRASH_ALERT, HANG_ALERT, SHELL_ALERT } from '../diagnostics/fixtures';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { Alerts } from './Alerts';

vi.mock('@tanstack/react-router', async (original) => ({
    ...(await original<typeof import('@tanstack/react-router')>()),
    Link: ({ to, className, children }: { to: string; className?: string; children: ReactNode }) => (
        <a href={to} className={className}>
            {children}
        </a>
    ),
}));

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));

/** The card for an account holding `permissions`, the alert list as the topic says. */
const card = async (alerts: Alert[], permissions = '1001') => {
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
                value={{ id: 2, username: 'kenji', isOwner: false, permissions, mustChangePassword: false }}
            >
                <LiveContext value={live}>
                    <AlertsProvider>
                        <Alerts />
                    </AlertsProvider>
                </LiveContext>
            </MeContext>
        </I18nProvider>,
    );
    live.start();
    await settle();
    const say = (message: unknown) => act(() => socket?.message(JSON.stringify(message)));
    say({ type: 'hello', connectionId: 1, contract: 1 });
    say({ type: 'snapshot', topic: 'server.alerts', data: { alerts, since: '2026-10-28T14:20:00Z' } });
    return { ...view, live, say };
};

/** Each event as it reads: its sentence, then its facts. */
const events = () =>
    [...document.querySelectorAll('li')].map((item) =>
        [...item.querySelectorAll('p')].map((line) => line.textContent),
    );

describe('Alerts', () => {
    afterEach(cleanup);

    it('should say what the tree started, how often, from what, and that it was left running', async () => {
        const { live } = await card([SHELL_ALERT]);

        const [[sentence, meta]] = events() as [[string, string]];
        expect(sentence).toBe(`${en.alerts.securityAlert}FXServer's tree started powershell.exe`);
        expect(meta).toMatch(/^\d\d:\d\d · 3 times · parent FXServer\.exe · left running, audited$/);
        live.stop();
    });

    it('should word a crash by its exception and module, and a hang by what svMain waited in', async () => {
        const { live } = await card([HANG_ALERT, CRASH_ALERT]);

        const [hang, crash] = events() as [[string, string], [string, string]];
        expect(hang[0]).toBe(`${en.alerts.hang}svMain was waiting in futex`);
        expect(hang[1]).toMatch(/· dump taken before the kill$/);
        expect(crash[0]).toBe(`${en.alerts.crash}Access violation in citizen-scripting-lua.dll`);
        expect(crash[1]).toMatch(/· 3rd time in 7 days · thread UV loop: svMain$/);
        live.stop();
    });

    it('should say how FXServer ended when no dump says more', async () => {
        const noDump: Alert = {
            ...CRASH_ALERT,
            crash: {
                ...CRASH_ALERT.crash!,
                exception: null,
                module: null,
                thread: null,
                exit: '0xC0000005',
                groupCount: 1,
            },
        };
        const busy: Alert = { ...HANG_ALERT, key: 'crash:9', crash: { ...HANG_ALERT.crash!, busy: true } };

        const { live } = await card([noDump, busy]);

        const [crash, hang] = events() as [[string, string], [string, string]];
        expect(crash[0]).toBe(`${en.alerts.crash}FXServer exited with exit code 0xC0000005`);
        expect(hang[0]).toBe(`${en.alerts.hang}svMain was busy on the CPU`);
        live.stop();
    });

    it('should open Diagnostics only for an account that may, and say why not to the others', async () => {
        const allowed = await card([SHELL_ALERT]);
        expect(allowed.getByRole('link', { name: en.alerts.open }).getAttribute('href')).toBe('/diagnostics');
        allowed.live.stop();
        cleanup();

        const { getByTitle, live } = await card([SHELL_ALERT], '1');

        const button = getByTitle(en.alerts.needsPermission);
        expect(button.getAttribute('aria-disabled')).toBe('true');
        expect(button.getAttribute('title')).toBe(en.alerts.needsPermission);
        live.stop();
    });

    it('should show nothing while no alert is live', async () => {
        const { container, live } = await card([]);

        expect(container.textContent).toBe('');
        live.stop();
    });
});
