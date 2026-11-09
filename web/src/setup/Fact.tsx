import type { ReactNode } from 'react';

import { cn } from '../lib/cn';

/**
 * A line of a `<dl>`: what it is, then its value, which wraps anywhere when it is a long path.
 * `compact` puts the value under its label, for a narrow column.
 */
export function Fact({
    label,
    compact = false,
    children,
}: {
    label: string;
    compact?: boolean;
    children: ReactNode;
}) {
    return (
        <div
            className={cn(
                'flex border-b border-line py-1.5 last:border-b-0',
                compact ? 'flex-col gap-0.5' : 'flex-wrap items-baseline gap-x-4 gap-y-0.5',
            )}
        >
            <dt className={cn('text-ui text-ink-3', !compact && 'w-32 flex-none')}>{label}</dt>
            <dd className="min-w-0 flex-1 text-body text-ink [overflow-wrap:anywhere]">{children}</dd>
        </div>
    );
}
