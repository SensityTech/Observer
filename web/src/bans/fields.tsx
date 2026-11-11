import { ChevronDown } from 'lucide-react';
import { useId, type ReactNode } from 'react';

import { characters, MAX_MINUTES, MAX_REASON } from '../api/bans';
import { FieldError, inputClass } from '../components/Field';
import { Segmented } from '../components/Segmented';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

export const LENGTHS = ['hour', 'day', 'threeDays', 'week', 'month', 'permanent', 'other'] as const;
export type Length = (typeof LENGTHS)[number];
type Usual = Exclude<Length, 'other'>;
const USUAL: Record<Usual, number | null> = {
    hour: 60,
    day: 24 * 60,
    threeDays: 3 * 24 * 60,
    week: 7 * 24 * 60,
    month: 30 * 24 * 60,
    permanent: null,
};
const UNITS = { minutes: 1, hours: 60, days: 24 * 60 } as const;
type Unit = keyof typeof UNITS;
const KEPT = ['keep', ...LENGTHS] as const;

/** A length as it is being picked. `keep` leaves a ban's end as it is, where the field offers it. */
export type LengthDraft = { length: Length | 'keep'; amount: string; unit: Unit };

export const A_DAY: LengthDraft = { length: 'day', amount: '2', unit: 'hours' };

/** The minutes a draft stands for, `null` for good; `undefined` for `keep` or a length no ban takes. */
export const draftMinutes = ({ length, amount, unit }: LengthDraft): number | null | undefined => {
    if (length === 'keep') {
        return undefined;
    }
    if (length !== 'other') {
        return USUAL[length];
    }
    const minutes = Math.round(Number(amount) * UNITS[unit]);
    return Number.isFinite(minutes) && minutes >= 1 && minutes <= MAX_MINUTES ? minutes : undefined;
};

/** The draft a stored length opens as: a usual one, or its largest unit that counts it whole. */
export const draftOf = (minutes: number | null): LengthDraft => {
    const usual = (Object.keys(USUAL) as Usual[]).find((each) => USUAL[each] === minutes);
    if (usual !== undefined || minutes === null) {
        return { ...A_DAY, length: usual ?? 'permanent' };
    }
    const unit = (['days', 'hours'] as const).find((each) => minutes % UNITS[each] === 0) ?? 'minutes';
    return { length: 'other', amount: String(minutes / UNITS[unit]), unit };
};

/** Whether a reason can be sent as it is typed: some text, within what the API keeps. */
export const reasonFits = (reason: string) => reason.trim() !== '' && characters(reason.trim()) <= MAX_REASON;

/** The usual lengths as a segmented choice, and an amount of minutes, hours or days for any other. */
export function LengthField({
    label,
    keep,
    draft,
    invalid,
    onChange,
}: {
    label: string;
    /** The word of a first choice that leaves the end as it is. */
    keep?: string;
    draft: LengthDraft;
    invalid: boolean;
    onChange: (draft: LengthDraft) => void;
}) {
    const { t } = useI18n();
    const words = t.bans.dialog;
    const amountId = useId();
    const unitId = useId();
    return (
        <>
            <Segmented
                label={label}
                choices={keep === undefined ? LENGTHS : KEPT}
                value={draft.length}
                words={{ ...words.lengths, keep: keep ?? '' }}
                onChange={(length) => onChange({ ...draft, length })}
            />
            {draft.length === 'other' && (
                <div className="flex flex-wrap items-end gap-2">
                    <div className="flex flex-col gap-1.5">
                        <label htmlFor={amountId} className="text-ui font-medium text-ink">
                            {words.amount}
                        </label>
                        <input
                            id={amountId}
                            type="number"
                            inputMode="numeric"
                            min={1}
                            className={cn(inputClass, 'w-28')}
                            aria-invalid={invalid || undefined}
                            value={draft.amount}
                            onChange={(event) => onChange({ ...draft, amount: event.target.value })}
                        />
                    </div>
                    <div className="flex flex-col gap-1.5">
                        <label htmlFor={unitId} className="text-ui font-medium text-ink">
                            {words.unit}
                        </label>
                        <div className="relative">
                            <select
                                id={unitId}
                                className={cn(inputClass, 'w-36 appearance-none pr-9')}
                                value={draft.unit}
                                onChange={(event) => onChange({ ...draft, unit: event.target.value as Unit })}
                            >
                                {(Object.keys(UNITS) as Unit[]).map((each) => (
                                    <option key={each} value={each}>
                                        {words.units[each]}
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
                </div>
            )}
        </>
    );
}

/** What stands under a length no ban takes: the range as a hint, as an error once the form was sent. */
export function NoSuchLength({ tried }: { tried: boolean }) {
    const { t } = useI18n();
    const range = t.bans.dialog.outOfRange;
    return tried ? <FieldError>{range}</FieldError> : <p className="text-small text-ink-3">{range}</p>;
}

/** A ban's reason, with how much of its length is used; the error replaces the hint. */
export function ReasonField({
    value,
    hint,
    error,
    onChange,
}: {
    value: string;
    hint: ReactNode;
    error?: string;
    onChange: (value: string) => void;
}) {
    const { t } = useI18n();
    const words = t.bans.dialog;
    const id = useId();
    return (
        <div className="flex flex-col gap-1.5">
            <div className="flex items-baseline justify-between gap-3">
                <label htmlFor={id} className="text-ui font-medium text-ink">
                    {words.reason}
                </label>
                <span className="font-mono text-meta text-ink-3">
                    {words.counter(characters(value.trim()), MAX_REASON)}
                </span>
            </div>
            <textarea
                id={id}
                rows={2}
                maxLength={MAX_REASON * 2}
                aria-invalid={error !== undefined || undefined}
                aria-describedby={`${id}-hint`}
                className={cn(inputClass, 'h-auto min-h-16 resize-y py-2.5 leading-[1.55]')}
                value={value}
                onChange={(event) => onChange(event.target.value)}
            />
            {error === undefined ? (
                <p id={`${id}-hint`} className="text-small text-ink-3">
                    {hint}
                </p>
            ) : (
                <FieldError id={`${id}-hint`}>{error}</FieldError>
            )}
        </div>
    );
}
