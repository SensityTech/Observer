import type { LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';

/** DESIGN.md's card: a head with its icon box and title, named by it, then the body. */
export function Card({
    id,
    icon: Icon,
    title,
    meta,
    aside,
    flush = false,
    children,
}: {
    id: string;
    icon: LucideIcon;
    title: string;
    meta?: string;
    /** Pushed to the end of the head: the card's own actions. */
    aside?: ReactNode;
    /** The body runs to the card's edges, as a table does. */
    flush?: boolean;
    children: ReactNode;
}) {
    return (
        <section className="card" aria-labelledby={id}>
            <div className="card-head">
                <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                    <Icon size={14} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <h2 id={id} className="text-card text-ink">
                    {title}
                </h2>
                {meta !== undefined && <p className="text-meta text-ink-3">{meta}</p>}
                {aside !== undefined && (
                    <div className="ml-auto flex flex-wrap items-center gap-2">{aside}</div>
                )}
            </div>
            <div className={flush ? '' : 'p-5 max-md:p-4'}>{children}</div>
        </section>
    );
}
