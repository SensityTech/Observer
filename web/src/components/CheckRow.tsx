import { useId } from 'react';

import { Checkbox } from './Checkbox';

/** A setting turned on or off, its name and what it does beside the box, the whole row pressable. */
export function CheckRow({
    label,
    description,
    checked,
    onChange,
}: {
    label: string;
    description: string;
    checked: boolean;
    onChange: (checked: boolean) => void;
}) {
    const id = useId();
    return (
        <label
            htmlFor={`${id}-box`}
            className="flex cursor-pointer items-start gap-3 rounded-sm px-2 py-2 hover:bg-hover"
        >
            <Checkbox
                id={`${id}-box`}
                checked={checked}
                aria-labelledby={`${id}-name`}
                aria-describedby={`${id}-about`}
                onChange={(event) => onChange(event.target.checked)}
            />
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span id={`${id}-name`} className="text-body font-medium text-ink">
                    {label}
                </span>
                <span id={`${id}-about`} className="text-small text-ink-2">
                    {description}
                </span>
            </span>
        </label>
    );
}
