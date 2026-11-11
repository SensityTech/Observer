import { cn } from '../lib/cn';

/**
 * A short exclusive choice as a segmented control (DESIGN §8.2): a group named by its legend, each
 * choice a pressed or unpressed button, 44 px on a phone.
 */
export function Segmented<Choice extends string>({
    label,
    choices,
    value,
    words,
    counts,
    onChange,
    className,
}: {
    /** What the choice is about, said to assistive technology. */
    label: string;
    choices: readonly Choice[];
    value: Choice;
    words: Record<Choice, string>;
    /** How many each choice shows, written after its word. */
    counts?: Record<Choice, string>;
    onChange: (choice: Choice) => void;
    className?: string;
}) {
    return (
        <fieldset
            className={cn(
                'inline-flex max-w-full flex-wrap gap-0.5 rounded-control border border-line bg-surface-2 p-0.75',
                className,
            )}
        >
            <legend className="sr-only">{label}</legend>
            {choices.map((choice) => (
                <button
                    key={choice}
                    type="button"
                    aria-pressed={value === choice}
                    onClick={() => onChange(choice)}
                    className="inline-flex h-6.5 items-center gap-1.5 rounded-[7px] px-2.5 text-small font-medium text-ink-3 transition-colors hover:text-ink focus-visible:outline-offset-1 aria-pressed:bg-raise aria-pressed:text-ink aria-pressed:shadow-[var(--elev-sm),0_0_0_1px_var(--line)] max-md:h-11 max-md:min-w-11 max-md:justify-center"
                >
                    {words[choice]}
                    {counts !== undefined && (
                        <span className="font-mono text-meta font-normal text-ink-3">{counts[choice]}</span>
                    )}
                </button>
            ))}
        </fieldset>
    );
}
