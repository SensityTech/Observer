import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, within, type RenderResult } from '@testing-library/react';
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

/** A phone, or a screen wide enough not to be one: jsdom answers no media query. */
const screenIs = (phone: boolean) =>
    vi.stubGlobal('matchMedia', (query: string) => ({ matches: phone && query === '(max-width: 760px)' }));

/**
 * The console of an operator who reads it, holding `count` lines. `meanwhile` is what the reader
 * does before the first lines come.
 */
const consoleWith = async (
    count: number,
    meanwhile: (view: RenderResult) => void = () => {},
    permissions = '9',
) => {
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
                        permissions,
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
    meanwhile(view);
    const lines = Array.from({ length: count }, (_, index) => line(index + 1));
    const snapshot = () =>
        socket?.message(
            JSON.stringify({
                type: 'snapshot',
                topic: 'console',
                data: { lines, gap: false, restarted: false, last: count },
            }),
        );
    act(() => {
        socket?.message(JSON.stringify({ type: 'hello', connectionId: 1, contract: 1 }));
        snapshot();
    });
    await frame();
    // By its place rather than its name, which counts the lines a paused view holds.
    const output = within(view.getByRole('region', { name: en.console.card })).getByRole('region');
    // Where the browser leaves the view when something above the card takes some of its height.
    Object.defineProperty(output, 'scrollHeight', { configurable: true, value: 1800 });
    output.scrollTop = 1200;
    const more = (batch: ConsoleLine[]) =>
        act(() =>
            socket?.message(JSON.stringify({ type: 'event', topic: 'console', data: { lines: batch } })),
        );
    return { ...view, live, output, more, resubscribed: () => act(snapshot) };
};

describe('ConsolePage', () => {
    const scrollIntoView = vi.fn<(this: Element, options?: ScrollIntoViewOptions) => void>();

    beforeEach(() => {
        vi.stubGlobal('ResizeObserver', TestResizeObserver);
        screenIs(false);
        // jsdom scrolls nothing: the test reads where the page was asked to go.
        Element.prototype.scrollIntoView = scrollIntoView;
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        observed.clear();
        scrollIntoView.mockReset();
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

    it('should scroll the page to the card on a phone when the first lines come, and only then', async () => {
        screenIs(true);

        const { getByRole, resubscribed, live } = await consoleWith(120);
        resubscribed();

        expect(scrollIntoView).toHaveBeenCalledOnce();
        expect(scrollIntoView.mock.contexts[0]).toBe(getByRole('region', { name: en.console.card }));
        expect(scrollIntoView).toHaveBeenCalledWith({ block: 'start' });
        live.stop();
    });

    it('should leave the page where it is on a wide screen', async () => {
        const { live } = await consoleWith(120);

        expect(scrollIntoView).not.toHaveBeenCalled();
        live.stop();
    });

    it('should leave the page where the reader moved it before the first lines came', async () => {
        screenIs(true);

        const { live } = await consoleWith(120, () => vi.stubGlobal('scrollY', 320));

        expect(scrollIntoView).not.toHaveBeenCalled();
        live.stop();
    });

    it('should leave the page alone when the reader paused before the first lines came', async () => {
        screenIs(true);

        const { live } = await consoleWith(120, ({ getByRole }) =>
            fireEvent.click(getByRole('button', { name: en.console.pause })),
        );

        expect(scrollIntoView).not.toHaveBeenCalled();
        live.stop();
    });

    it('should scroll the page to the card again when the reader jumps to the newest line', async () => {
        screenIs(true);
        const { output, more, findByRole, live } = await consoleWith(120);
        scrollIntoView.mockClear();
        fireEvent.scroll(output);

        await more([line(121), line(122)]);
        expect(scrollIntoView).not.toHaveBeenCalled();
        fireEvent.click(await findByRole('button', { name: en.console.newLines(2) }));

        expect(scrollIntoView).toHaveBeenCalledOnce();
        live.stop();
    });
});

const typeAndSend = (field: HTMLElement, command: string) => {
    fireEvent.change(field, { target: { value: command } });
    fireEvent.submit(field.closest('form') as HTMLFormElement);
};

describe('ConsolePage command line', () => {
    /** The console commands Observer was sent, each answered when the test says. */
    const sent: { command: string; answer: (delivery: string) => void }[] = [];

    beforeEach(() => {
        vi.stubGlobal('ResizeObserver', TestResizeObserver);
        screenIs(false);
        Element.prototype.scrollIntoView = () => {};
        vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
            if (url.endsWith('/console') && init?.method === 'POST') {
                const body = JSON.parse(String(init.body)) as { line: string };
                const delivery = await new Promise<string>((answer) =>
                    sent.push({ command: body.line, answer }),
                );
                return Response.json({ delivery, masked: false });
            }
            return Response.json({ code: 'not_found' }, { status: 404 });
        });
    });

    afterEach(() => {
        cleanup();
        vi.unstubAllGlobals();
        observed.clear();
        sent.length = 0;
        localStorage.clear();
    });

    it('should keep what is typed while a command is on its way, and send both in order', async () => {
        const { getByRole, live } = await consoleWith(0, () => {}, '7');
        const field = getByRole('textbox', { name: en.console.commandLabel });

        typeAndSend(field, 'status');
        expect(field).toHaveProperty('value', '');
        typeAndSend(field, 'players');
        fireEvent.change(field, { target: { value: 'restart hello' } });
        await settle();
        sent[0]?.answer('written');
        await settle();
        sent[1]?.answer('written');
        await settle();

        expect(sent.map(({ command }) => command)).toEqual(['status', 'players']);
        expect(field).toHaveProperty('value', 'restart hello');
        live.stop();
    });

    it('should give a refused command back to the field when nothing else was typed', async () => {
        const { getByRole, live } = await consoleWith(0, () => {}, '7');
        const field = getByRole('textbox', { name: en.console.commandLabel });

        typeAndSend(field, 'status');
        await settle();
        sent[0]?.answer('busy');
        await settle();

        expect(field).toHaveProperty('value', 'status');
        expect(getByRole('alert').textContent).toBe(en.console.busy);
        live.stop();
    });
});
