import type { LucideIcon } from 'lucide-react';
import { useEffect, useId, useRef, type KeyboardEvent, type ReactNode } from 'react';

import { cn } from '../lib/cn';

export type MenuItem = {
    label: string;
    icon: LucideIcon;
    onSelect: () => void;
    danger?: boolean;
};

type MenuProps = {
    /** The button's accessible name. */
    label: string;
    buttonClassName: string;
    /** What the button shows. */
    children: ReactNode;
    items: MenuItem[];
    /** below-end under a toolbar button; above-stretch over the rail's footer, at its width. */
    placement: 'below-end' | 'above-stretch';
};

const GAP = 6;

/**
 * A menu in the browser's top layer (the popover attribute), so no scrolling card clips it. The
 * browser closes it on Escape and on a click outside.
 */
export function Menu({ label, buttonClassName, children, items, placement }: MenuProps) {
    const id = useId();
    const menu = useRef<HTMLDivElement>(null);
    const button = useRef<HTMLButtonElement>(null);

    useEffect(() => {
        const popover = menu.current;
        if (popover === null) {
            return;
        }
        const close = () => popover.hidePopover();
        const viewport = document.documentElement;
        // Placed before it shows, so it never paints a frame at the browser's default spot.
        const place = (event: Event) => {
            const anchor = button.current;
            if (!(event instanceof ToggleEvent) || event.newState !== 'open' || anchor === null) {
                return;
            }
            const box = anchor.getBoundingClientRect();
            if (placement === 'below-end') {
                popover.style.top = `${box.bottom + GAP}px`;
                popover.style.right = `${viewport.clientWidth - box.right}px`;
            } else {
                popover.style.bottom = `${viewport.clientHeight - box.top + GAP}px`;
                popover.style.left = `${box.left}px`;
                popover.style.width = `${box.width}px`;
            }
        };
        const follow = (event: Event) => {
            if (!(event instanceof ToggleEvent)) {
                return;
            }
            if (event.newState === 'open') {
                popover.querySelector<HTMLElement>('[role="menuitem"]')?.focus();
                // A fixed menu would drift away from its button: it closes instead.
                window.addEventListener('scroll', close, true);
                window.addEventListener('resize', close);
            } else {
                window.removeEventListener('scroll', close, true);
                window.removeEventListener('resize', close);
            }
        };
        popover.addEventListener('beforetoggle', place);
        popover.addEventListener('toggle', follow);
        return () => {
            popover.removeEventListener('beforetoggle', place);
            popover.removeEventListener('toggle', follow);
            window.removeEventListener('scroll', close, true);
            window.removeEventListener('resize', close);
        };
    }, [placement]);

    const moveFocus = (event: KeyboardEvent<HTMLDivElement>) => {
        const entries = [...(menu.current?.querySelectorAll<HTMLElement>('[role="menuitem"]') ?? [])];
        const current = entries.indexOf(document.activeElement as HTMLElement);
        const target = {
            ArrowDown: entries[(current + 1) % entries.length],
            ArrowUp: entries[(current - 1 + entries.length) % entries.length],
            Home: entries[0],
            End: entries.at(-1),
        }[event.key];
        if (target !== undefined) {
            event.preventDefault();
            target.focus();
        }
    };

    const select = (item: MenuItem) => {
        menu.current?.hidePopover();
        button.current?.focus();
        item.onSelect();
    };

    return (
        <>
            <button
                ref={button}
                type="button"
                className={buttonClassName}
                aria-label={label}
                aria-haspopup="menu"
                aria-controls={id}
                popoverTarget={id}
            >
                {children}
            </button>
            <div
                ref={menu}
                id={id}
                popover="auto"
                role="menu"
                tabIndex={-1}
                onKeyDown={moveFocus}
                className="fixed inset-auto m-0 min-w-60 rounded-box border border-line-strong bg-surface p-1.5 text-ink shadow-pop open:animate-pop"
            >
                {items.map((item) => (
                    <button
                        key={item.label}
                        type="button"
                        role="menuitem"
                        className={cn(
                            'flex min-h-8.5 w-full items-center gap-2.5 rounded-sm px-2.5 text-left text-nav hover:bg-hover focus-visible:bg-hover focus-visible:outline-none max-md:min-h-11',
                            item.danger
                                ? 'text-danger-ink'
                                : 'text-ink-2 hover:text-ink focus-visible:text-ink [&>svg]:text-ink-3',
                        )}
                        onClick={() => select(item)}
                    >
                        <item.icon size={16} strokeWidth={1.5} aria-hidden="true" />
                        {item.label}
                    </button>
                ))}
            </div>
        </>
    );
}
