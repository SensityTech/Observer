import type { ReactNode } from 'react';

import type { Messages } from '../i18n/en';

/** An exception in words: the catalog's name for its code, else the analysis's own name in mono. */
export const exceptionWords = (t: Messages, code: string, name: string): ReactNode =>
    Object.hasOwn(t.diagnostics.exceptions, code) ? (
        t.diagnostics.exceptions[code]
    ) : (
        <span className="font-mono">{name}</span>
    );

/** FXServer's main thread by the name the panel uses for it everywhere else. */
export const threadWord = (thread: string | null) =>
    thread === null || thread.includes('svMain') ? 'svMain' : thread;

/** How FXServer ended in words: a signal by its name, or `exit code 0xC0000005`. */
export const exitWords = (t: Messages, exitCode: string | null, signal: number | null) => {
    if (signal !== null) {
        return t.alerts.signals[signal] ?? t.alerts.signal(signal);
    }
    return exitCode === null ? null : t.alerts.exitCode(exitCode);
};

/** A module or a file, as code. */
export function Code({ children }: { children: ReactNode }) {
    return <code className="code-inline [overflow-wrap:anywhere]">{children}</code>;
}
