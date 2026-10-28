// The commands a user sent, kept in this browser for ↑ and ↓. Never sent anywhere: the audit log
// is the record, this is a convenience, and a browser that refuses storage simply forgets.

const MAX = 100;

const key = (username: string) => `observer.console.history.${username}`;

export const readHistory = (username: string): string[] => {
    try {
        const stored: unknown = JSON.parse(localStorage.getItem(key(username)) ?? '[]');
        return Array.isArray(stored) ? stored.filter((entry) => typeof entry === 'string').slice(-MAX) : [];
    } catch {
        return [];
    }
};

/** The history with `command` as its newest entry, saved. The same command twice in a row counts once. */
export const remember = (username: string, history: string[], command: string): string[] => {
    const next = history.at(-1) === command ? history : [...history, command].slice(-MAX);
    try {
        localStorage.setItem(key(username), JSON.stringify(next));
    } catch {
        // Storage full or refused: the history lasts as long as the page.
    }
    return next;
};

/** Where ↑ and ↓ stand: `index` in the history, or null while the field holds what was being typed. */
export type Position = { index: number | null; draft: string };

export const START: Position = { index: null, draft: '' };

/** ↑: the command before the one shown, keeping what was typed for ↓ to bring back. */
export const older = (history: string[], position: Position, typed: string): [Position, string] => {
    if (history.length === 0) {
        return [position, typed];
    }
    if (position.index === null) {
        const index = history.length - 1;
        return [{ index, draft: typed }, history[index] ?? typed];
    }
    const index = Math.max(0, position.index - 1);
    return [{ ...position, index }, history[index] ?? typed];
};

/** ↓: the command after the one shown, then what was being typed. */
export const newer = (history: string[], position: Position, typed: string): [Position, string] => {
    if (position.index === null) {
        return [position, typed];
    }
    if (position.index >= history.length - 1) {
        return [START, position.draft];
    }
    const index = position.index + 1;
    return [{ ...position, index }, history[index] ?? typed];
};
