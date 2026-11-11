import { ChevronDown } from 'lucide-react';
import { useId } from 'react';

import { cn } from '../lib/cn';
import { inputClass } from './Field';

/** A labelled choice among a few, on the native select drawn as the panel's fields. */
export function SelectField<Value extends string>({
    label,
    value,
    choices,
    onChange,
    narrowed = false,
    className,
}: {
    label: string;
    value: Value;
    choices: readonly { value: Value; label: string }[];
    onChange: (value: Value) => void;
    /** The choice narrows what the page shows: its border says so. */
    narrowed?: boolean;
    className?: string;
}) {
    const id = useId();
    return (
        <div className={cn('flex min-w-0 flex-col gap-1.5', className)}>
            <label htmlFor={id} className="text-ui font-medium text-ink">
                {label}
            </label>
            <div className="relative">
                <select
                    id={id}
                    className={cn(inputClass, 'appearance-none truncate pr-9', narrowed && 'border-ink-3')}
                    value={value}
                    onChange={(event) => {
                        const picked = choices.find((choice) => choice.value === event.target.value);
                        if (picked !== undefined) {
                            onChange(picked.value);
                        }
                    }}
                >
                    {choices.map((choice) => (
                        <option key={choice.value} value={choice.value}>
                            {choice.label}
                        </option>
                    ))}
                </select>
                <ChevronDown
                    size={16}
                    strokeWidth={1.5}
                    className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2 text-ink-3"
                    aria-hidden="true"
                />
            </div>
        </div>
    );
}
