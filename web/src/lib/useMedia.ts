import { useSyncExternalStore } from 'react';

/** The phone layout, as tokens.css's max-md: draws it. */
export const PHONE = '(max-width: 760px)';
/** Wide enough for two panes side by side, as 2xl: draws them. */
export const WIDE = '(min-width: 1181px)';

/** Whether a media query matches now, followed as the window changes. False where none is answered. */
export const useMedia = (query: string) =>
    useSyncExternalStore(
        (changed) => {
            if (typeof matchMedia !== 'function') {
                return () => {};
            }
            const list = matchMedia(query);
            list.addEventListener('change', changed);
            return () => list.removeEventListener('change', changed);
        },
        () => typeof matchMedia === 'function' && matchMedia(query).matches,
    );
