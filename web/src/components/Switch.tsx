import type { ComponentProps } from 'react';

/** The native checkbox as a switch: a 36 × 20 track, its knob moving over when on. */
export function Switch({
    checked,
    ...props
}: Omit<ComponentProps<'input'>, 'type' | 'role'> & { checked: boolean }) {
    return (
        <span className="relative mt-0.5 inline-flex h-5 w-9 flex-none">
            <input
                type="checkbox"
                role="switch"
                aria-checked={checked}
                checked={checked}
                className="peer absolute inset-0 m-0 cursor-pointer opacity-0 max-md:-inset-3"
                {...props}
            />
            <span
                aria-hidden="true"
                className="pointer-events-none absolute inset-0 rounded-pill border border-line-strong bg-surface-3 transition-colors peer-checked:border-btn peer-checked:bg-btn peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-accent peer-disabled:opacity-45"
            />
            <span
                aria-hidden="true"
                className="pointer-events-none absolute top-[3px] left-[3px] size-3.5 rounded-pill bg-ink-3 transition-[transform,background-color] duration-200 peer-checked:translate-x-4 peer-checked:bg-btn-ink"
            />
        </span>
    );
}
