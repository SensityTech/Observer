import { Check, Minus } from 'lucide-react';
import type { ReactNode } from 'react';

import { cn } from '../lib/cn';

/** A filter or an option turned on and off: a check when on, a dash when off, never the fill alone. */
export function Toggle({
    pressed,
    onChange,
    className,
    children,
}: {
    pressed: boolean;
    onChange: (pressed: boolean) => void;
    className?: string;
    children: ReactNode;
}) {
    const Icon = pressed ? Check : Minus;
    return (
        <button
            type="button"
            aria-pressed={pressed}
            onClick={() => onChange(!pressed)}
            className={cn(
                'inline-flex h-7 items-center gap-1.5 rounded-pill border border-line pr-2.5 pl-2 text-meta whitespace-nowrap text-ink-3 transition-colors hover:border-line-strong hover:text-ink aria-pressed:border-line-strong aria-pressed:bg-surface-3 aria-pressed:text-ink max-md:h-11',
                className,
            )}
        >
            <Icon size={12} strokeWidth={2} aria-hidden="true" />
            {children}
        </button>
    );
}

/** A short exclusive choice, as the charts' ranges are. */
export function Segmented<T extends string>({
    label,
    value,
    options,
    onChange,
}: {
    label: string;
    value: T;
    options: [T, string][];
    onChange: (value: T) => void;
}) {
    return (
        <fieldset className="inline-flex max-w-full gap-0.5 rounded-control border border-line bg-surface-2 p-0.75">
            <legend className="sr-only">{label}</legend>
            {options.map(([option, word]) => (
                <button
                    key={option}
                    type="button"
                    aria-pressed={value === option}
                    onClick={() => onChange(option)}
                    className="inline-flex h-6.5 items-center rounded-[7px] px-2.5 text-small font-medium text-ink-3 transition-colors hover:text-ink focus-visible:outline-offset-1 aria-pressed:bg-raise aria-pressed:text-ink aria-pressed:shadow-[var(--elev-sm),0_0_0_1px_var(--line)] max-md:h-11 max-md:min-w-11 max-md:justify-center"
                >
                    {word}
                </button>
            ))}
        </fieldset>
    );
}
