import { clsx, type ClassValue } from 'clsx';
import { extendTailwindMerge } from 'tailwind-merge';

// The custom theme names of tokens.css, so conflicting classes passed in className resolve.
// Without them, tailwind-merge takes `text-ui` for a colour and drops it next to `text-ink-3`.
const twMerge = extendTailwindMerge({
    extend: {
        theme: {
            text: [
                'tag',
                'micro',
                'eyebrow',
                'caption',
                'meta',
                'small',
                'ui',
                'nav',
                'body',
                'card',
                'touch',
                'figure-sm',
                'state',
                'figure',
                'page',
                'tile',
            ],
            radius: ['xs', 'sm', 'control', 'box', 'card', 'dialog', 'rail', 'pill'],
            shadow: ['sm', 'btn', 'pop', 'rail', 'lg'],
            tracking: ['label', 'group', 'tag', 'btn', 'snug', 'logo', 'title'],
            container: ['desc', 'search', 'split', 'toast', 'empty', 'drawer', 'dialog', 'palette'],
            animate: ['live-ping', 'pop', 'rise', 'slide', 'fade', 'skeleton'],
            ease: ['std'],
        },
    },
});

export const cn = (...inputs: ClassValue[]) => twMerge(clsx(inputs));
