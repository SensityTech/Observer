import { CircleAlert, Eye, EyeOff } from 'lucide-react';
import { useId, useState, type ComponentProps, type ReactNode } from 'react';

import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { buttonVariants } from './Button';

export const inputClass = cn(
    'block h-9.5 w-full rounded-control border border-line-strong bg-surface px-3 text-body text-ink outline-none transition-[border-color,box-shadow] placeholder:text-ink-3',
    'hover:border-[color-mix(in_srgb,var(--ink)_26%,transparent)] focus:border-accent focus:ring-3 focus:ring-focus-ring',
    'aria-invalid:border-danger aria-invalid:ring-[color-mix(in_srgb,var(--danger)_22%,transparent)]',
    'disabled:cursor-not-allowed disabled:bg-surface-2 disabled:text-ink-3 max-md:h-11 max-md:text-touch',
);

export function FieldError({ id, children }: { id?: string; children: ReactNode }) {
    return (
        <p id={id} role="alert" className="flex items-center gap-1.5 text-small text-danger-ink">
            <CircleAlert size={14} strokeWidth={1.5} className="flex-none" aria-hidden="true" />
            {children}
        </p>
    );
}

type FieldProps = ComponentProps<'input'> & {
    label: ReactNode;
    /** Sits between the label and the field, where the sign-in puts its help toggle. */
    labelAside?: ReactNode;
    hint?: ReactNode;
    error?: ReactNode;
    /** More elements the field is described by, such as a message shared by two fields. */
    describedBy?: string;
    inputClassName?: string;
    /** Inside the field's box, after the input: the password's reveal button. */
    after?: ReactNode;
    /** Under the field's box, before its error and hint. */
    below?: ReactNode;
    /** Beside the field's box, on its line: the folder field's Browse button. Under it on a phone. */
    beside?: ReactNode;
};

/** A labelled input with its hint and error, wired for assistive technology. */
export function TextField({
    id: given,
    label,
    labelAside,
    hint,
    error,
    describedBy,
    inputClassName,
    after,
    below,
    beside,
    className,
    ...input
}: FieldProps) {
    const generated = useId();
    const id = given ?? generated;
    const errorId = error ? `${id}-error` : undefined;
    // The error says what the hint would, more precisely: one of them at a time.
    const hintId = hint === undefined || errorId ? undefined : `${id}-hint`;
    const described = [errorId, hintId, describedBy].filter(Boolean).join(' ') || undefined;
    const invalid = Boolean(error) || input['aria-invalid'] === true;
    const box = (
        <>
            <input
                {...input}
                id={id}
                aria-invalid={invalid || undefined}
                aria-describedby={described}
                className={cn(inputClass, inputClassName)}
            />
            {after}
        </>
    );

    return (
        <div className={cn('flex min-w-0 flex-col gap-1.5', className)}>
            <div className="flex items-baseline justify-between gap-3">
                <label htmlFor={id} className="text-ui font-medium text-ink">
                    {label}
                </label>
                {labelAside}
            </div>
            {beside === undefined ? (
                <div className="relative">{box}</div>
            ) : (
                <div className="flex items-center gap-2 max-md:flex-col max-md:items-stretch">
                    <div className="relative min-w-0 flex-1">{box}</div>
                    {beside}
                </div>
            )}
            {below}
            {error && <FieldError id={errorId}>{error}</FieldError>}
            {hintId && (
                <p id={hintId} className="text-small text-ink-3">
                    {hint}
                </p>
            )}
        </div>
    );
}

/** A password field with a button that shows what was typed. */
export function PasswordField({ inputClassName, ...props }: Omit<FieldProps, 'type' | 'after'>) {
    const { t } = useI18n();
    const [shown, setShown] = useState(false);
    const Icon = shown ? EyeOff : Eye;

    return (
        <TextField
            {...props}
            type={shown ? 'text' : 'password'}
            spellCheck={false}
            autoCapitalize="none"
            inputClassName={cn('pr-12', inputClassName)}
            after={
                <button
                    type="button"
                    className={cn(
                        buttonVariants({ size: 'icon-sm', tone: 'ghost' }),
                        'absolute top-1/2 right-1 -translate-y-1/2 active:-translate-y-1/2 max-md:size-9',
                    )}
                    aria-label={t.common.showPassword}
                    aria-pressed={shown}
                    onClick={() => setShown((value) => !value)}
                >
                    <Icon size={16} strokeWidth={1.5} aria-hidden="true" />
                </button>
            }
        />
    );
}
