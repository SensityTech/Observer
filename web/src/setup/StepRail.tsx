import { Check } from 'lucide-react';

import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { SETUP_STEPS } from './state';

/**
 * The setup's steps down the side, each with its number and what it does, the passed ones ticked.
 * Where the setup is too narrow for it, one line says which step this is.
 */
export function StepRail({ current }: { current: number }) {
    const { t, format } = useI18n();
    const words = t.serverSetup;
    const step = SETUP_STEPS[current] ?? 'folder';
    return (
        <>
            <ol aria-label={words.steps} className="flex flex-col @max-[50rem]:hidden">
                {SETUP_STEPS.map((key, index) => {
                    const passed = index < current;
                    const here = index === current;
                    return (
                        <li
                            key={key}
                            aria-current={here ? 'step' : undefined}
                            className="relative flex items-start gap-3 pb-5.5 last:pb-0"
                        >
                            {index < SETUP_STEPS.length - 1 && (
                                <span
                                    className="absolute top-7.5 bottom-1.5 left-3 w-px bg-line-strong"
                                    aria-hidden="true"
                                />
                            )}
                            <span
                                className={cn(
                                    'grid size-6 flex-none place-items-center rounded-full border font-mono text-caption font-semibold tabular-nums',
                                    here
                                        ? 'border-transparent bg-btn text-btn-ink'
                                        : passed
                                          ? 'border-transparent bg-success-soft text-success'
                                          : 'border-line-strong bg-surface text-ink-3',
                                )}
                                aria-hidden="true"
                            >
                                {passed ? <Check size={12} strokeWidth={2.5} /> : format.number(index + 1)}
                            </span>
                            <span className="flex min-w-0 flex-col">
                                <span
                                    className={cn(
                                        'text-body',
                                        here ? 'font-semibold text-ink' : 'text-ink-2',
                                    )}
                                >
                                    {words.stepNames[key]}
                                    {passed && <span className="sr-only"> {words.said.passed}</span>}
                                </span>
                                <span className="text-small text-ink-3">{words.stepHints[key]}</span>
                            </span>
                        </li>
                    );
                })}
            </ol>
            <p className="text-ui font-medium text-ink @min-[50rem]:hidden">
                {words.stepOf(current + 1, SETUP_STEPS.length, words.stepNames[step])}
            </p>
        </>
    );
}
