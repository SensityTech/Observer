import { Check } from 'lucide-react';
import type { ComponentProps } from 'react';

import { cn } from '../lib/cn';

/**
 * The native checkbox, kept for keyboard and forms, drawn as the design's 18px box. `indeterminate`
 * draws a dash: some of what it stands for is checked.
 */
export function Checkbox({
    className,
    indeterminate = false,
    ...props
}: Omit<ComponentProps<'input'>, 'type' | 'ref'> & { indeterminate?: boolean }) {
    return (
        <span className={cn('relative mt-px inline-grid size-4.5 flex-none place-items-center', className)}>
            <input
                type="checkbox"
                ref={(input) => {
                    if (input !== null) {
                        input.indeterminate = indeterminate;
                    }
                }}
                className="peer absolute inset-0 m-0 cursor-pointer opacity-0 disabled:cursor-not-allowed"
                {...props}
            />
            <span
                aria-hidden="true"
                className="pointer-events-none grid size-4.5 place-items-center rounded-[5px] border border-line-strong bg-surface text-btn-ink transition-[background-color,border-color] peer-checked:border-btn peer-checked:bg-btn peer-indeterminate:border-btn peer-indeterminate:bg-btn peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-accent peer-disabled:opacity-45 peer-checked:[&>svg]:opacity-100 peer-indeterminate:[&>i]:opacity-100"
            >
                <Check
                    size={12}
                    strokeWidth={2.5}
                    className="col-start-1 row-start-1 opacity-0 transition-opacity"
                />
                <i className="col-start-1 row-start-1 h-0.5 w-2 rounded-full bg-btn-ink opacity-0" />
            </span>
        </span>
    );
}
