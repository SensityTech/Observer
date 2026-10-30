import { Circle, CircleCheck, CircleX, LoaderCircle } from 'lucide-react';

import { useI18n } from '../i18n/I18nProvider';
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
    numbered = false,
    said,
    compact,
}: {
    label: string;
    steps: Step[];
    /** -1 when none is. */
    current: number;
    busy: boolean;
    /** Each step shows its place, as the steps of a form do. */
    numbered?: boolean;
    /** What a screen reader hears of a step that passed or failed, which the icon only shows. */
    said?: { passed: string; failed: string };
    /** What a phone reads in place of the list: `Step 2 of 4 · Config file`. */
    compact?: string;
}) {
    const { format } = useI18n();
    return (
        <>
            <ol
                className={cn('flex flex-wrap gap-x-4 gap-y-2', compact !== undefined && 'max-md:hidden')}
                aria-label={label}
            >
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
                                isCurrent
                                    ? 'font-medium text-ink'
                                    : step.passed
                                      ? 'text-ink-2'
                                      : 'text-ink-3',
                                step.failed && 'text-danger-ink',
                            )}
                        >
                            {numbered && (
                                <span className="font-mono text-small tabular-nums">
                                    {format.number(index + 1)}
                                </span>
                            )}
                            <Icon
                                size={14}
                                strokeWidth={1.75}
                                className={cn(isCurrent && busy && 'animate-spin')}
                                aria-hidden="true"
                            />
                            {step.label}
                            {said !== undefined && (step.passed || step.failed) && (
                                <span className="sr-only"> {step.failed ? said.failed : said.passed}</span>
                            )}
                        </li>
                    );
                })}
            </ol>
            {compact !== undefined && <p className="text-ui font-medium text-ink md:hidden">{compact}</p>}
        </>
    );
}
