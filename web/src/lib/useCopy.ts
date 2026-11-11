import { useEffect, useState } from 'react';

/** How long "Copied" stays before the button offers to copy again. */
const COPIED_FOR = 2_000;

export type CopyState = 'idle' | 'copied' | 'blocked';

/**
 * Copies a text in one click where the browser allows it. `copied` goes back to `idle` after a
 * moment, unless it is `kept`; `blocked` stays, for whoever shows the text to offer it another way.
 */
export const useCopy = ({ kept = false }: { kept?: boolean } = {}) => {
    const [state, setState] = useState<CopyState>('idle');
    useEffect(() => {
        if (state !== 'copied' || kept) {
            return;
        }
        const timer = setTimeout(() => setState('idle'), COPIED_FOR);
        return () => clearTimeout(timer);
    }, [state, kept]);
    /** Whether the text was copied. */
    const copy = async (text: string) => {
        try {
            await navigator.clipboard.writeText(text);
            setState('copied');
            return true;
        } catch {
            // No clipboard outside a secure context (plain HTTP on a LAN address).
            setState('blocked');
            return false;
        }
    };
    return { state, copy };
};
