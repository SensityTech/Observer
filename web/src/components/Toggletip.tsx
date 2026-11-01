import { useEffect, useId, useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';

import { cn } from '../lib/cn';

/** How long the tip waits once the pointer left, so that it can cross the gap into the tip. */
const LEAVE_DELAY = 150;
/** The gap between the trigger and the tip, and the least margin to the window's sides. */
const GAP = 6;
const MARGIN = 8;

type Open = 'hover' | 'pinned' | null;

/**
 * A word or a chip that tells more: hovering shows the tip, which stays while the pointer is on
 * either; a click or Enter pins it open, a second one closes it, and so do Escape and a click
 * elsewhere. The tip describes its trigger even closed, so a screen reader reads it on focus. On a
 * phone the trigger takes 44 px of height to tap without growing.
 */
export function Toggletip({
    tip,
    className,
    children,
}: {
    /** What the tip says. */
    tip: ReactNode;
    /** The trigger's own classes, a chip's for instance. */
    className?: string;
    /** What the trigger shows: its accessible name. */
    children: ReactNode;
}) {
    const id = useId();
    const trigger = useRef<HTMLButtonElement>(null);
    const box = useRef<HTMLDivElement>(null);
    const leaving = useRef<number | undefined>(undefined);
    const [open, setOpen] = useState<Open>(null);
    const [at, setAt] = useState<{ top: number; left: number }>({ top: 0, left: 0 });

    const stay = () => window.clearTimeout(leaving.current);
    const show = (how: Exclude<Open, null>) => {
        stay();
        const rect = trigger.current?.getBoundingClientRect();
        if (rect !== undefined) {
            setAt({ top: rect.bottom + GAP, left: rect.left });
        }
        setOpen(how);
    };
    const close = () => {
        stay();
        setOpen(null);
    };
    const leave = () => {
        if (open === 'hover') {
            stay();
            leaving.current = window.setTimeout(() => setOpen(null), LEAVE_DELAY);
        }
    };

    useEffect(() => () => window.clearTimeout(leaving.current), []);

    // The pointer on the tip keeps it, as on the trigger: it can be hovered and read, unlike a title.
    useEffect(() => {
        const element = box.current;
        if (element === null || open !== 'hover') {
            return;
        }
        const keep = () => window.clearTimeout(leaving.current);
        const go = () => {
            window.clearTimeout(leaving.current);
            leaving.current = window.setTimeout(() => setOpen(null), LEAVE_DELAY);
        };
        element.addEventListener('mouseenter', keep);
        element.addEventListener('mouseleave', go);
        return () => {
            element.removeEventListener('mouseenter', keep);
            element.removeEventListener('mouseleave', go);
        };
    }, [open]);

    // Kept inside the window's width, whatever the trigger's place.
    useLayoutEffect(() => {
        const element = box.current;
        if (open === null || element === null) {
            return;
        }
        const room = document.documentElement.clientWidth - MARGIN;
        element.style.left = `${Math.max(MARGIN, Math.min(at.left, room - element.offsetWidth))}px`;
    }, [open, at]);

    useEffect(() => {
        if (open === null) {
            return;
        }
        const escape = (event: KeyboardEvent) => {
            if (event.key === 'Escape') {
                setOpen(null);
            }
        };
        const elsewhere = (event: PointerEvent) => {
            const target = event.target as Node | null;
            if (!trigger.current?.contains(target) && !box.current?.contains(target)) {
                setOpen(null);
            }
        };
        // A fixed tip would drift away from its trigger: it closes instead.
        const drift = () => setOpen(null);
        window.addEventListener('keydown', escape);
        window.addEventListener('pointerdown', elsewhere);
        window.addEventListener('scroll', drift, true);
        window.addEventListener('resize', drift);
        return () => {
            window.removeEventListener('keydown', escape);
            window.removeEventListener('pointerdown', elsewhere);
            window.removeEventListener('scroll', drift, true);
            window.removeEventListener('resize', drift);
        };
    }, [open]);

    return (
        <>
            <button
                ref={trigger}
                type="button"
                aria-expanded={open !== null}
                aria-controls={id}
                aria-describedby={id}
                className={cn(
                    'relative cursor-help focus-visible:outline-offset-1 max-md:after:absolute max-md:after:inset-x-0 max-md:after:-inset-y-2.75',
                    className,
                )}
                onClick={() => (open === 'pinned' ? close() : show('pinned'))}
                onMouseEnter={() => (open === null ? show('hover') : stay())}
                onMouseLeave={leave}
            >
                {children}
            </button>
            {/* Out of the heading or the table it sits in, which would read it or clip it. */}
            {createPortal(
                <div
                    ref={box}
                    id={id}
                    role="tooltip"
                    hidden={open === null}
                    className={cn(
                        'fixed z-(--z-popover) w-max max-w-72 flex-col gap-0.5 rounded-sm border border-line-strong bg-surface px-2.5 py-1.5 text-meta text-ink-2 shadow-pop',
                        open === null ? 'hidden' : 'flex',
                    )}
                    style={{ top: at.top, left: at.left }}
                >
                    {tip}
                </div>,
                document.body,
            )}
        </>
    );
}
