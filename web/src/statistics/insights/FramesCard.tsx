import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { TrendingDown, TrendingUp } from 'lucide-react';

import type { DropsRange } from '../../api/generated/DropsRange';
import type { FrameDistribution } from '../../api/generated/FrameDistribution';
import { framesQuery } from '../../api/statistics';
import { Chip } from '../../components/Chip';
import { signedPercent, formatShare } from '../../drops/words';
import { formatTimestamp } from '../../i18n/format';
import { useI18n } from '../../i18n/I18nProvider';
import { cn } from '../../lib/cn';
import { CardState, InsightCard } from './InsightCard';

/** A p99 this much better or worse than before is worth a tone; under it, about the same. */
const TELLING = 0.15;
const TALLEST = 170;

/** The intervals of the bounds as the axis names them: `< 1`, `1–2`, …, `> 250`. */
export const intervalLabels = (
    bounds: readonly number[],
    number: (value: number) => string,
    words: {
        under: (ms: string) => string;
        beyond: (ms: string) => string;
        interval: (from: string, to: string) => string;
    },
) =>
    Array.from({ length: bounds.length + 1 }, (_, index) => {
        if (index === 0) {
            return words.under(number(bounds[0] ?? 0));
        }
        if (index === bounds.length) {
            return words.beyond(number(bounds[index - 1] ?? 0));
        }
        return words.interval(number(bounds[index - 1] ?? 0), number(bounds[index] ?? 0));
    });

/** How p99 moved, as the chip says it: better or worse past 15 %, about the same under. */
export const p99Change = (distribution: FrameDistribution) => {
    const now = distribution.now.p99;
    const before = distribution.before.p99;
    if (now === null || before === null || before <= 0) {
        return null;
    }
    const change = now / before - 1;
    return { change, tone: change <= -TELLING ? 'better' : change >= TELLING ? 'worse' : 'same' } as const;
};

export function FramesCard({ range }: { range: DropsRange }) {
    const { t, locale, format } = useI18n();
    const f = t.statistics.frames;
    const query = useQuery({ ...framesQuery(range), placeholderData: keepPreviousData });
    const data = query.data;
    const now = data?.now;
    const shares = now?.shares ?? null;
    const moved = data === undefined ? null : p99Change(data);
    const chip =
        moved === null ? undefined : moved.tone === 'same' ? (
            <Chip>{f.same}</Chip>
        ) : (
            <Chip
                tone={moved.tone === 'better' ? 'success' : 'warning'}
                icon={moved.tone === 'better' ? TrendingDown : TrendingUp}
            >
                {(moved.tone === 'better' ? f.better : f.worse)(signedPercent(locale, moved.change))}
            </Chip>
        );
    const foot =
        now?.overBudget == null
            ? undefined
            : f.underBudget(formatShare(locale, 1 - now.overBudget), format.number(data?.budget ?? 50));

    return (
        <InsightCard
            title={f.title}
            meta={f.meta(t.drops.ranges[range])}
            chip={chip}
            foot={foot}
            stale={query.isPlaceholderData}
        >
            {data === undefined || shares === null ? (
                <CardState pending={query.isPending} error={query.error} failed={f.failed} empty={f.empty} />
            ) : (
                <Bars distribution={data} shares={shares} />
            )}
        </InsightCard>
    );
}

function Bars({ distribution, shares }: { distribution: FrameDistribution; shares: number[] }) {
    const { t, locale, format } = useI18n();
    const f = t.statistics.frames;
    const before = distribution.before.shares;
    const labels = intervalLabels(distribution.bounds, (value) => format.number(value), f);
    const budgetAt = distribution.bounds.findIndex((bound) => bound === distribution.budget);
    const top = Math.max(...shares, ...(before ?? [0]), 0.0001);
    const share = (value: number) => formatShare(locale, value);
    const ms = (value: number | null) =>
        value === null ? t.statistics.instant.unknown : format.milliseconds(value);
    return (
        <figure className="flex flex-col gap-2.5">
            <div
                className="relative flex h-[210px] items-end gap-1 border-b border-line-strong max-md:gap-0.5"
                aria-hidden="true"
            >
                {shares.map((value, index) => {
                    const height = Math.max(2, (value / top) * TALLEST);
                    const ghost =
                        before === null ? null : Math.max(2, ((before[index] ?? 0) / top) * TALLEST);
                    const over = budgetAt !== -1 && index > budgetAt;
                    return (
                        <div key={labels[index]} className="relative h-full flex-1">
                            {ghost !== null && (
                                <div
                                    className="absolute inset-x-[12%] bottom-0 rounded-t-[4px] border border-b-0 border-dashed border-ink-3"
                                    style={{ height: ghost }}
                                />
                            )}
                            <div
                                className={cn(
                                    'absolute inset-x-[12%] bottom-0 rounded-t-[4px]',
                                    over ? 'bg-warning-fill' : 'bg-accent',
                                )}
                                style={{ height }}
                            />
                            <span
                                className="absolute inset-x-0 text-center font-mono text-tag text-ink-2 max-md:hidden"
                                style={{ bottom: Math.max(height, ghost ?? 0) + 4 }}
                            >
                                {value >= 0.001 ? share(value) : ''}
                            </span>
                        </div>
                    );
                })}
                {budgetAt !== -1 && (
                    <div
                        className="absolute inset-y-0 border-l border-dashed border-ink-3"
                        style={{ left: `${((budgetAt + 1) / shares.length) * 100}%` }}
                    >
                        <span className="absolute top-0 left-1.5 font-mono text-micro whitespace-nowrap text-ink-3">
                            {t.statistics.timeline.keys.budget(`${format.number(distribution.budget)} ms`)}
                        </span>
                    </div>
                )}
            </div>
            <div className="flex gap-1 max-md:gap-0.5" aria-hidden="true">
                {labels.map((label, index) => (
                    <span key={label} className="flex-1 text-center font-mono text-tag text-ink-3">
                        {index % 2 === 0 || shares.length <= 9 ? label : ''}
                    </span>
                ))}
            </div>
            <ul className="sr-only">
                {shares.map((value, index) => (
                    <li key={labels[index]}>
                        {f.bar(
                            labels[index] ?? '',
                            share(value),
                            before === null ? null : share(before[index] ?? 0),
                        )}
                    </li>
                ))}
            </ul>
            <figcaption className="flex flex-wrap items-center gap-x-4 gap-y-1.5 text-small">
                <span className="flex items-center gap-1.5 text-ink-3">
                    <span className="size-2.5 rounded-[2px] bg-accent" aria-hidden="true" />
                    {f.now}
                </span>
                <span className="flex items-center gap-1.5 text-ink-3">
                    <span
                        className="size-2.5 rounded-[2px] border border-dashed border-ink-3"
                        aria-hidden="true"
                    />
                    {f.before}
                </span>
                <span className="flex items-center gap-1.5 text-ink-3">
                    <span className="size-2.5 rounded-[2px] bg-warning-fill" aria-hidden="true" />
                    {f.overBudget}
                </span>
                <span className="font-mono text-meta text-ink-2 md:ml-auto">
                    {f.figures(ms(distribution.now.p50), ms(distribution.now.p95), ms(distribution.now.p99))}
                </span>
            </figcaption>
            {before === null && <p className="text-small text-ink-3">{f.noBefore}</p>}
            {distribution.since !== null && distribution.since > distribution.before.from && (
                <p className="text-small text-ink-3">
                    {f.since(formatTimestamp(new Date(distribution.since * 1000), { seconds: false }))}
                </p>
            )}
        </figure>
    );
}
