import { useLocation } from '@tanstack/react-router';
import { Clock } from 'lucide-react';
import { useEffect, useId, useRef, type ReactNode } from 'react';

import { Chip } from '../components/Chip';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { APPLIES, anchorOf, type CategoryId, type SettingId } from './catalog';

/** The ids a row's control is named and described by. */
export type RowIds = { control: string; label: string; about: string };

/**
 * A setting: its name, what it does in plain words and when a change applies, beside its control.
 * A `field` row names its control with a `<label>`; the others name a group of controls, which
 * points at `label` itself. The row a search led to is marked and brought into view.
 */
export function SettingRow({
    id,
    field = false,
    changed = false,
    children,
}: {
    id: SettingId;
    field?: boolean;
    changed?: boolean;
    children: (ids: RowIds) => ReactNode;
}) {
    const { t } = useI18n();
    const base = useId();
    const ids: RowIds = { control: `${base}-control`, label: `${base}-label`, about: `${base}-about` };
    const anchor = anchorOf(id);
    const found = useLocation({ select: (location) => location.hash === anchor });
    const row = useRef<HTMLDivElement>(null);
    const { label, about } = t.settings.items[id];
    const applies = APPLIES.get(id);

    useEffect(() => {
        if (found) {
            row.current?.scrollIntoView({ block: 'center' });
        }
    }, [found]);

    return (
        <div
            ref={row}
            id={anchor}
            className={cn(
                'flex flex-wrap items-start gap-x-6 gap-y-3 border-t border-line px-5 py-4 first:border-t-0 max-md:px-4',
                // ink-3 falls under 4.5:1 on the accent's tint: the row's grey text, hints included, turns ink-2.
                found && 'bg-accent-soft [&_.text-ink-3]:text-ink-2',
            )}
        >
            <div className="flex min-w-0 flex-[1_1_16rem] flex-col gap-1">
                <div className="flex flex-wrap items-center gap-2">
                    {field ? (
                        <label
                            id={ids.label}
                            htmlFor={ids.control}
                            className="text-body font-medium text-ink"
                        >
                            {label}
                        </label>
                    ) : (
                        <span id={ids.label} className="text-body font-medium text-ink">
                            {label}
                        </span>
                    )}
                    {changed && <Chip tone="warning">{t.settings.changed}</Chip>}
                </div>
                <p id={ids.about} className="text-small text-pretty text-ink-3">
                    {about}
                </p>
                {applies !== undefined && (
                    <p className="flex items-center gap-1.5 text-meta text-ink-3">
                        <Clock size={12} strokeWidth={2} aria-hidden="true" />
                        {t.settings.applies[applies]}
                    </p>
                )}
            </div>
            <div className="flex min-w-0 flex-[1_1_22rem] flex-col gap-2">{children(ids)}</div>
        </div>
    );
}

/** A card of settings under its own heading, the rows running to its edges. */
export function SettingsGroup({
    title,
    aside,
    children,
}: {
    title: string;
    aside?: ReactNode;
    children: ReactNode;
}) {
    const id = useId();
    return (
        <section className="card overflow-hidden" aria-labelledby={id}>
            <div className="card-head">
                <h3 id={id} className="text-card text-ink">
                    {title}
                </h3>
                {aside !== undefined && (
                    <div className="ml-auto flex flex-wrap items-center gap-2">{aside}</div>
                )}
            </div>
            <div className="flex flex-col">{children}</div>
        </section>
    );
}

/** A notice of a group, above its rows. */
export function GroupNotice({ children }: { children: ReactNode }) {
    return <div className="flex flex-col gap-2 px-5 pt-4 max-md:px-4">{children}</div>;
}

/** The category's heading, what it holds, and at its end a way to the page that acts on it. */
export function CategoryHead({ category, aside }: { category: CategoryId; aside?: ReactNode }) {
    const { t } = useI18n();
    const { name, about } = t.settings.categories[category];
    return (
        <div className="flex flex-wrap items-end justify-between gap-x-4 gap-y-3">
            <div className="flex max-w-desc min-w-0 flex-col gap-1">
                <h2 className="text-figure-sm font-semibold text-ink">{name}</h2>
                <p className="text-small text-pretty text-ink-2">{about}</p>
            </div>
            {aside}
        </div>
    );
}

/** A short exclusive choice drawn as a segmented control, named by its row. */
export function Segmented<T extends string>({
    ids,
    name,
    choices,
    words,
    value,
    onChange,
}: {
    ids: RowIds;
    name: string;
    choices: readonly T[];
    words: Record<T, string>;
    value: T;
    onChange: (value: T) => void;
}) {
    return (
        <div
            role="radiogroup"
            aria-labelledby={ids.label}
            aria-describedby={ids.about}
            className="inline-flex max-w-full flex-wrap gap-0.5 self-start rounded-control border border-line bg-surface-2 p-0.75"
        >
            {choices.map((choice) => (
                <label
                    key={choice}
                    className="inline-flex h-6.5 cursor-pointer items-center justify-center rounded-[7px] px-2.5 text-small font-medium text-ink-3 transition-colors hover:text-ink has-checked:bg-raise has-checked:text-ink has-checked:shadow-[var(--elev-sm),0_0_0_1px_var(--line)] has-focus-visible:outline-2 has-focus-visible:outline-offset-1 has-focus-visible:outline-accent has-disabled:cursor-not-allowed has-disabled:opacity-45 max-md:h-11 max-md:min-w-11 max-md:px-3.5"
                >
                    <input
                        type="radio"
                        name={name}
                        value={choice}
                        className="sr-only"
                        checked={value === choice}
                        onChange={() => onChange(choice)}
                    />
                    {words[choice]}
                </label>
            ))}
        </div>
    );
}
