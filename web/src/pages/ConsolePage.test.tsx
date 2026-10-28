import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ConsoleLine } from '../api/generated/ConsoleLine';
import { MeContext } from '../auth/me';
import { en } from '../i18n/en';
import { I18nProvider } from '../i18n/I18nProvider';
import { LiveContext } from '../live/context';
import { Realtime, type SocketEvents } from '../live/realtime';
import { ServerStatusProvider } from '../server/context';
import { ConsolePage } from './ConsolePage';

const settle = () => act(() => new Promise((resolve) => setTimeout(resolve, 0)));
const frame = () => act(() => new Promise((resolve) => requestAnimationFrame(resolve)));

/** jsdom lays nothing out: the test says when an element changed size. */
const observed = new Map<Element, Set<ResizeObserverCallback>>();
class TestResizeObserver {
    readonly #callback: ResizeObserverCallback;
    readonly #targets = new Set<Element>();

    constructor(callback: ResizeObserverCallback) {
        this.#callback = callback;
    }

    observe(target: Element) {
        this.#targets.add(target);
        const callbacks = observed.get(target) ?? new Set();
        callbacks.add(this.#callback);
        observed.set(target, callbacks);
    }

    unobserve(target: Element) {
        this.#targets.delete(target);
        observed.get(target)?.delete(this.#callback);
    }

    disconnect() {
        for (const target of this.#targets) {
            observed.get(target)?.delete(this.#callback);
        }
        this.#targets.clear();
    }
}
const resized = (target: Element) =>
    act(() => {
        for (const callback of observed.get(target) ?? []) {
            callback([], new TestResizeObserver(() => {}));
        }
    });

const line = (seq: number): ConsoleLine => [seq, Date.now(), 1, 'stdout', null, `line ${seq}`, 0];

/** The console of an operator who reads it, holding `count` lines. */
const consoleWith = async (count: number) => {
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
        <QueryClientProvider client={new QueryClient()}>
            <I18nProvider>
                <MeContext
                    value={{
                        id: 2,
                        username: 'kenji',
                        isOwner: false,
                        permissions: '9',
                        mustChangePassword: false,
                    }}
                >
                    <LiveContext value={live}>
                        <ServerStatusProvider>
                            <ConsolePage />
                        </ServerStatusProvider>
                    </LiveContext>
                </MeContext>
            </I18nProvider>
        </QueryClientProvider>,
    );
    live.start();
    await settle();
    const lines = Array.from({ length: count }, (_, index) => line(index + 1));
    act(() => {
        socket?.message(JSON.stringify({ type: 'hello', connectionId: 1, contract: 1 }));
        socket?.message(
            JSON.stringify({
                type: 'snapshot',
                topic: 'console',
                data: { lines, gap: false, restarted: false, last: count },
            }),
        );
    });
    await frame();
    const output = view.getByRole('region', { name: en.console.output(count) });
    // Where the browser leaves the view when something above the card takes some of its height.
    Object.defineProperty(output, 'scrollHeight', { configurable: true, value: 1800 });
    output.scrollTop = 1200;
    return { ...view, live, output };
};

describe('ConsolePage', () => {
    beforeEach(() => {
        vi.stubGlobal('ResizeObserver', TestResizeObserver);
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        observed.clear();
    });

    it('should keep the newest line in view when the output shrinks while it follows', async () => {
        const { output, live } = await consoleWith(120);

        await resized(output);

        expect(output.scrollTop).toBe(1800);
        live.stop();
    });

    it('should leave the view where the reader scrolled it when the output shrinks', async () => {
        const { output, live } = await consoleWith(120);
        fireEvent.scroll(output);

        await resized(output);

        expect(output.scrollTop).toBe(1200);
        live.stop();
    });
});
