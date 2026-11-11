import { useNavigate } from '@tanstack/react-router';
import type { ComponentProps } from 'react';

/**
 * A link to a place on the documentation's page. Not the router's `Link`: every one of them leads
 * to the page that is open, and the router would mark them all as the current page.
 */
export function Anchor({ id, onClick, children, ...props }: { id: string } & ComponentProps<'a'>) {
    const navigate = useNavigate();
    return (
        <a
            {...props}
            href={`#${id}`}
            onClick={(event) => {
                onClick?.(event);
                // A click that opens another tab or window is the browser's.
                if (
                    event.defaultPrevented ||
                    event.button !== 0 ||
                    event.metaKey ||
                    event.ctrlKey ||
                    event.shiftKey ||
                    event.altKey
                ) {
                    return;
                }
                event.preventDefault();
                void navigate({ to: '/docs', hash: id });
                // Also when the address already names it: the reader asked to be taken there again.
                document.getElementById(id)?.scrollIntoView();
            }}
        >
            {children}
        </a>
    );
}
