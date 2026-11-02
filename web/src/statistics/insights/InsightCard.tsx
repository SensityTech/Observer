import { useId, type ReactNode } from 'react';

import { isFeatureDisabled } from '../../api/statistics';
import { errorMessage } from '../../i18n/errors';
import { useI18n } from '../../i18n/I18nProvider';
import { cn } from '../../lib/cn';

/** An insight's card: its title and what it reads, a chip on the right, the body, a sentence under it. */
export function InsightCard({
    title,
    meta,
    chip,
    foot,
    stale = false,
    className,
    children,
}: {
    title: string;
    meta: string;
    chip?: ReactNode;
    foot?: ReactNode;
    stale?: boolean;
    className?: string;
    children: ReactNode;
}) {
    const titleId = useId();
    return (
        <section className={cn('card flex min-w-0 flex-col', className)} aria-labelledby={titleId}>
            <div className="card-head items-start">
                <div className="flex min-w-0 flex-col gap-0.5">
                    <h2 id={titleId} className="card-title">
                        {title}
                    </h2>
                    <span className="text-small text-ink-3">{meta}</span>
                </div>
                {chip !== undefined && <span className="ml-auto">{chip}</span>}
            </div>
            <div
                className={cn(
                    'card-body flex flex-1 flex-col gap-3 transition-opacity max-md:p-4',
                    stale && 'opacity-50',
                )}
                aria-busy={stale}
            >
                {children}
            </div>
            {foot !== undefined && (
                <div className="rounded-b-card border-t border-line bg-bg-subtle px-5 py-3 text-small text-ink-2 max-md:px-4">
                    {foot}
                </div>
            )}
        </section>
    );
}

/** What a card says when it has nothing to draw: loading, a failure, turned off, or no data. */
export function CardState({
    pending,
    error,
    failed,
    empty,
}: {
    pending: boolean;
    error: unknown;
    failed: string;
    empty: string | null;
}) {
    const { t } = useI18n();
    if (pending) {
        return <div className="skeleton h-[210px] w-full" aria-busy="true" />;
    }
    const text = isFeatureDisabled(error)
        ? t.statistics.offCard
        : error !== null && error !== undefined
          ? `${failed} ${errorMessage(t, error)}`
          : empty;
    return text === null ? null : (
        <div className="grid min-h-[210px] place-items-center rounded-card border border-dashed border-line-strong bg-bg-subtle px-6 text-center text-ui text-ink-3">
            {text}
        </div>
    );
}

/** Seven day names from Monday, as the browser's language writes them short. */
export const weekdays = (locale: string) =>
    Array.from({ length: 7 }, (_, day) =>
        new Intl.DateTimeFormat(locale, { weekday: 'short', timeZone: 'UTC' }).format(
            Date.UTC(2024, 0, 1 + day),
        ),
    );

/** A share of the accent for a value out of `max`, so that a cell's shade reads its size. */
export const shade = (value: number, max: number) =>
    `color-mix(in srgb, var(--accent) ${Math.round(8 + 88 * Math.min(1, value / max))}%, transparent)`;

/** The legend of a shaded map: five steps from fewer to more, never a gradient. */
export function Scale({ fewer, more }: { fewer: string; more: string }) {
    return (
        <span className="flex items-center gap-2" aria-hidden="true">
            {fewer}
            <span className="flex gap-0.5">
                {[0, 0.25, 0.5, 0.75, 1].map((step) => (
                    <span
                        key={step}
                        className="size-2.5 rounded-[2px]"
                        style={{ background: shade(step, 1) }}
                    />
                ))}
            </span>
            {more}
        </span>
    );
}
