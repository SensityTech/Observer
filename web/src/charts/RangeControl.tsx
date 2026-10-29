import type { Range } from '../api/generated/Range';
import { useI18n } from '../i18n/I18nProvider';

const RANGES: Range[] = ['1h', '6h', '24h', '7d', '30d'];

/** The ranges a chart reads, as a segmented control in its card's head, named after its chart. */
export function RangeControl({
    label,
    value,
    onChange,
}: {
    label: string;
    value: Range;
    onChange: (range: Range) => void;
}) {
    const { t } = useI18n();
    return (
        <fieldset className="ml-auto inline-flex max-w-full gap-0.5 rounded-control border border-line bg-surface-2 p-0.75 md:-my-1.5">
            <legend className="sr-only">{label}</legend>
            {RANGES.map((choice) => (
                <button
                    key={choice}
                    type="button"
                    aria-pressed={value === choice}
                    onClick={() => onChange(choice)}
                    className="inline-flex h-6.5 items-center rounded-[7px] px-2.5 text-small font-medium text-ink-3 transition-colors hover:text-ink focus-visible:outline-offset-1 aria-pressed:bg-raise aria-pressed:text-ink aria-pressed:shadow-[var(--elev-sm),0_0_0_1px_var(--line)] max-md:h-11 max-md:min-w-11 max-md:justify-center"
                >
                    {t.charts.ranges[choice]}
                </button>
            ))}
        </fieldset>
    );
}
