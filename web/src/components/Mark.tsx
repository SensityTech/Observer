import { useId } from 'react';

import { cn } from '../lib/cn';

/**
 * Observer's mark on its 24-unit grid: a ring in the text colour and the accent bead on it, top
 * right, with the ring cleared around the bead (`--mark-bead` recolours it). `cargo xtask icons`
 * draws the same shapes for the executable. The ring starts at the bead, so a stroke drawn in
 * grows out of it.
 */
export function Mark({ size, className }: { size?: number; className?: string }) {
    const clear = `mark-clear-${useId().replace(/[^\w-]/g, '')}`;
    return (
        <svg
            viewBox="0 0 24 24"
            width={size}
            height={size}
            className={cn('mark', className)}
            aria-hidden="true"
        >
            <mask id={clear}>
                <rect width="24" height="24" fill="white" />
                <circle className="mark-bead" cx="17.303" cy="6.697" r="4.5" />
            </mask>
            <path
                className="mark-ring"
                d="M17.303 6.697A7.5 7.5 0 0 1 6.697 17.303A7.5 7.5 0 0 1 17.303 6.697"
                fill="none"
                stroke="currentColor"
                strokeWidth="3"
                pathLength={1}
                mask={`url(#${clear})`}
            />
            <circle
                className="mark-bead"
                cx="17.303"
                cy="6.697"
                r="3"
                fill="var(--mark-bead, var(--accent))"
            />
        </svg>
    );
}
