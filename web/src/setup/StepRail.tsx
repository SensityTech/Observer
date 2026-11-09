import { Check } from 'lucide-react';

import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { SETUP_STEPS, type SetupStep } from './state';

/**
 * The setup's steps down the side: the current one raised, the passed ones ticked with what they
 * chose, the others with what they are for.
 */
export function StepRail({
    current,
    chosen,
}: {
    current: number;
    chosen: Partial<Record<SetupStep, string>>;
}) {
    const { t, format } = useI18n();
    const words = t.serverSetup;
    return (
        <ol aria-label={words.steps} className="flex flex-col gap-1">
            {SETUP_STEPS.map((key, index) => {
                const passed = index < current;
                const here = index === current;
                const value = chosen[key];
                return (
                    <li
                        key={key}
                        aria-current={here ? 'step' : undefined}
                        className={cn('flex items-start gap-3 rounded-box p-3', here && 'bg-surface-2')}
                    >
                        <span
                            className={cn(
                                'grid size-6.5 flex-none place-items-center rounded-full font-mono text-caption font-semibold tabular-nums',
                                here
                                    ? 'bg-btn text-btn-ink'
                                    : passed
                                      ? 'bg-success-soft text-success'
                                      : 'border border-line-strong text-ink-3',
                            )}
                            aria-hidden="true"
                        >
                            {passed ? <Check size={13} strokeWidth={3} /> : format.number(index + 1)}
                        </span>
                        <span className="flex min-w-0 flex-col gap-0.5">
                            <span className={cn('text-body', here ? 'font-semibold text-ink' : 'text-ink-2')}>
                                {words.stepNames[key]}
                                {passed && <span className="sr-only"> {words.said.passed}</span>}
                            </span>
                            {passed && value !== undefined ? (
                                <span className="truncate font-mono text-caption text-ink-3" title={value}>
                                    {value}
                                </span>
                            ) : (
                                <span className="text-small text-ink-3">{words.stepHints[key]}</span>
                            )}
                        </span>
                    </li>
                );
            })}
        </ol>
    );
}
