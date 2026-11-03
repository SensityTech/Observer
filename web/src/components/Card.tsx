import type { LucideIcon } from 'lucide-react';
import { createContext, createElement, use, type ReactNode, type Ref } from 'react';

/** The level of card titles: one below a page's own sections, as in a category of the settings. */
export const CardHeading = createContext<'h2' | 'h3'>('h2');

/** The panel's card: a head with its icon box and title, named by it, then the body. */
export function Card({
    id,
    icon: Icon,
    title,
    titleRef,
    meta,
    aside,
    flush = false,
    children,
}: {
    id: string;
    icon: LucideIcon;
    title: string;
    /** The keyboard is moved to the title when what it was on went away. */
    titleRef?: Ref<HTMLHeadingElement>;
    meta?: string;
    /** Pushed to the end of the head: the card's own actions. */
    aside?: ReactNode;
    /** The body runs to the card's edges, as a table does. */
    flush?: boolean;
    children: ReactNode;
}) {
    const heading = use(CardHeading);
    return (
        <section className="card" aria-labelledby={id}>
            <div className="card-head">
                <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                    <Icon size={14} strokeWidth={1.5} aria-hidden="true" />
                </span>
                {createElement(
                    heading,
                    {
                        id,
                        ref: titleRef,
                        tabIndex: titleRef === undefined ? undefined : -1,
                        className: 'text-card text-ink',
                    },
                    title,
                )}
                {meta !== undefined && <p className="text-meta text-ink-3">{meta}</p>}
                {aside !== undefined && (
                    <div className="ml-auto flex flex-wrap items-center gap-2">{aside}</div>
                )}
            </div>
            <div className={flush ? '' : 'p-5 max-md:p-4'}>{children}</div>
        </section>
    );
}
