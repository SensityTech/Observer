import type { ReactNode } from 'react';

/** A line of a `<dl>`: what it is, then its value, which wraps anywhere when it is a long path. */
export function Fact({ label, children }: { label: string; children: ReactNode }) {
    return (
        <div className="flex flex-wrap items-baseline gap-x-4 gap-y-0.5 border-b border-line py-1.5 last:border-b-0">
            <dt className="w-32 flex-none text-ui text-ink-3">{label}</dt>
            <dd className="min-w-0 flex-1 text-body text-ink [overflow-wrap:anywhere]">{children}</dd>
        </div>
    );
}
