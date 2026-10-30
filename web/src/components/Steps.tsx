import { Circle, CircleCheck, CircleX, LoaderCircle } from 'lucide-react';

import { cn } from '../lib/cn';

export type Step = { key: string; label: string; passed: boolean; failed: boolean };

/**
 * Where a sequence stands: each step an icon and a word, so that no state rests on colour alone.
 * The current step carries `aria-current`, and a spinner while it is under way.
 */
export function Steps({
    label,
    steps,
    current,
    busy,
}: {
    label: string;
    steps: Step[];
    current: number;
    busy: boolean;
}) {
    return (
        <ol className="flex flex-wrap gap-x-4 gap-y-2" aria-label={label}>
            {steps.map((step, index) => {
                const isCurrent = index === current;
                const Icon = step.failed
                    ? CircleX
                    : step.passed
                      ? CircleCheck
                      : isCurrent && busy
                        ? LoaderCircle
                        : Circle;
                return (
                    <li
                        key={step.key}
                        aria-current={isCurrent ? 'step' : undefined}
                        className={cn(
                            'inline-flex items-center gap-1.5 text-ui',
                            isCurrent ? 'font-medium text-ink' : step.passed ? 'text-ink-2' : 'text-ink-3',
                            step.failed && 'text-danger-ink',
                        )}
                    >
                        <Icon
                            size={14}
                            strokeWidth={1.75}
                            className={cn(isCurrent && busy && 'animate-spin')}
                            aria-hidden="true"
                        />
                        {step.label}
                    </li>
                );
            })}
        </ol>
    );
}
