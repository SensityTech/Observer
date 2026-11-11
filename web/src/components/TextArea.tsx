import { useId, type ComponentProps, type ReactNode } from 'react';

import { cn } from '../lib/cn';
import { FieldError, inputClass } from './Field';

type TextAreaProps = ComponentProps<'textarea'> & {
    label: ReactNode;
    /** Beside the label: how much of the text's length is used. */
    counter?: ReactNode;
    hint?: ReactNode;
    error?: ReactNode;
};

/** A labelled text of a few lines, with its counter, hint and error, wired for assistive technology. */
export function TextArea({ label, counter, hint, error, className, ...textarea }: TextAreaProps) {
    const id = useId();
    const errorId = error ? `${id}-error` : undefined;
    // The error says what the hint would, more precisely: one of them at a time.
    const hintId = hint === undefined || errorId ? undefined : `${id}-hint`;
    return (
        <div className="flex min-w-0 flex-col gap-1.5">
            <div className="flex items-baseline justify-between gap-3">
                <label htmlFor={id} className="text-ui font-medium text-ink">
                    {label}
                </label>
                {counter !== undefined && <span className="font-mono text-meta text-ink-3">{counter}</span>}
            </div>
            <textarea
                {...textarea}
                id={id}
                aria-invalid={Boolean(error) || undefined}
                aria-describedby={errorId ?? hintId}
                className={cn(inputClass, 'h-auto min-h-16 resize-y py-2.5 leading-[1.55]', className)}
            />
            {error && <FieldError id={errorId}>{error}</FieldError>}
            {hintId && (
                <p id={hintId} className="text-small text-ink-3">
                    {hint}
                </p>
            )}
        </div>
    );
}
