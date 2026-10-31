import { ChartBar } from 'lucide-react';
import { Fragment, type ReactNode } from 'react';

import type { CategoryShare } from '../api/generated/CategoryShare';
import type { DropCategory } from '../api/generated/DropCategory';
import type { DropsSummary } from '../api/generated/DropsSummary';
import { Card } from '../components/Card';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { chartOf } from './search';
import { CategoryWord, formatRate, formatShare, ShareBar, trendWindows, VerdictChip } from './words';

/**
 * The selected row (DESIGN §8.8): the accent's soft fill, its 2 px bar against the first cell, and
 * the secondary text in ink-2, which ink-3 would not be readable on.
 */
const SELECTED_ROW =
    'bg-accent-soft [&_.text-ink-3]:text-ink-2 [&>td:first-child]:shadow-[inset_2px_0_0_var(--accent)]';
/** The same on a phone's line, the bar against the line itself. */
const SELECTED_ITEM = 'bg-accent-soft shadow-[inset_2px_0_0_var(--accent)] [&_.text-ink-3]:text-ink-2';

/** Unexpected categories first, then the expected ones, each ranked by its drops. */
const groups = (categories: readonly CategoryShare[]) =>
    ([false, true] as const)
        .map((expected) => ({
            expected,
            rows: categories
                .filter((category) => category.expected === expected)
                .toSorted((a, b) => b.drops - a.drops),
        }))
        .filter((group) => group.rows.length > 0);

/**
 * The categories as a ranked table with their share of every drop, their rate and its verdict, the
 * unexpected ones apart from the expected ones. A category narrows the log, for who sees it, and the
 * chart when the chart shows it; one that would do neither is not a button.
 */
export function BreakdownCard({
    summary,
    failed,
    stale,
    selected,
    onSelect,
    withLog,
}: {
    summary: DropsSummary | undefined;
    failed: boolean;
    /** Still the previous range's, while the new one is read. */
    stale: boolean;
    selected: DropCategory | undefined;
    onSelect: (category: DropCategory | undefined) => void;
    withLog: boolean;
}) {
    const { t, format } = useI18n();
    const words = t.drops.breakdown;
    let body: ReactNode;
    if (summary === undefined) {
        body = failed ? (
            <div className="p-5 max-md:p-4">
                <Notice tone="danger" live={false}>
                    {words.failed}
                </Notice>
            </div>
        ) : (
            <div className="flex flex-col gap-3 p-5" aria-busy="true">
                <span className="sr-only">{words.loading}</span>
                <span className="skeleton w-3/4" />
                <span className="skeleton w-2/3" />
                <span className="skeleton w-1/2" />
            </div>
        );
    } else if (summary.categories.every((row) => row.drops === 0)) {
        body = <p className="px-6 py-4 text-center text-ui text-ink-3">{words.empty}</p>;
    } else {
        body = <Ranking summary={summary} selected={selected} onSelect={onSelect} withLog={withLog} />;
    }
    return (
        <Card
            id="drops-breakdown"
            icon={ChartBar}
            title={words.title}
            meta={
                summary === undefined
                    ? undefined
                    : words.meta(summary.sessions, format.playTime(summary.playedMs))
            }
            flush
        >
            <div aria-busy={stale} className={cn('transition-opacity', stale && 'opacity-50')}>
                {body}
            </div>
            <p className="rounded-b-card bg-bg-subtle px-5 py-2.5 text-meta text-ink-3 max-md:px-4">
                {withLog ? words.filterHint : words.filterHintChart}
            </p>
        </Card>
    );
}

function Ranking({
    summary,
    selected,
    onSelect,
    withLog,
}: {
    summary: DropsSummary;
    selected: DropCategory | undefined;
    onSelect: (category: DropCategory | undefined) => void;
    withLog: boolean;
}) {
    const { t, locale, format } = useI18n();
    const words = t.drops.breakdown;
    const windows = trendWindows(t, locale, summary);
    const ranked = groups(summary.categories);
    // The table's column names the unit; a phone's line says it.
    const rate = (row: CategoryShare, unit: boolean) => {
        if (row.rate === null) {
            return <span className="text-ink-3">{words.notPlayers}</span>;
        }
        const figure = formatRate(locale, row.rate);
        return unit ? t.drops.rate(figure) : figure;
    };
    const name = (row: CategoryShare) => {
        const word = <CategoryWord category={row.category} />;
        if (!withLog && chartOf(row.category) === undefined) {
            return word;
        }
        return (
            <button
                type="button"
                aria-pressed={selected === row.category}
                onClick={() => onSelect(selected === row.category ? undefined : row.category)}
                className="-mx-1 self-start rounded-xs px-1 text-left hover:underline max-md:min-h-11"
            >
                {word}
            </button>
        );
    };
    const loading = (row: CategoryShare) =>
        row.loadingShare === null ? null : (
            <span className="text-meta text-ink-3">
                {words.loadingShare(formatShare(locale, row.loadingShare))}
            </span>
        );
    return (
        <>
            <div className={cn(table.scroll, 'rounded-none max-md:hidden')}>
                <table className={table.table}>
                    <caption className="sr-only">{words.caption}</caption>
                    <thead>
                        <tr>
                            <th scope="col" className={table.th}>
                                {words.category}
                            </th>
                            <th scope="col" className={cn(table.th, 'text-right')}>
                                {words.drops}
                            </th>
                            <th scope="col" className={cn(table.th, 'w-[26%]')}>
                                <span className="sr-only">{words.share}</span>
                            </th>
                            <th scope="col" className={cn(table.th, 'text-right')}>
                                {words.rate}
                            </th>
                            <th scope="col" className={table.th}>
                                {words.verdict}
                            </th>
                        </tr>
                    </thead>
                    {ranked.map((group) => (
                        <tbody key={String(group.expected)}>
                            <tr>
                                <th
                                    scope="rowgroup"
                                    colSpan={5}
                                    className="h-8 border-b border-line bg-bg-subtle px-5 text-left eyebrow max-md:px-4"
                                >
                                    {group.expected ? words.expected : words.unexpected}
                                </th>
                            </tr>
                            {group.rows.map((row, index) => (
                                <tr
                                    key={row.category}
                                    className={
                                        selected === row.category ? SELECTED_ROW : 'hover:bg-surface-2'
                                    }
                                >
                                    <td className={cn(table.td, 'align-top')}>
                                        <div className="flex flex-col gap-0.5">
                                            {name(row)}
                                            {loading(row)}
                                        </div>
                                    </td>
                                    <td
                                        className={cn(
                                            table.td,
                                            'w-[1%] text-right align-top font-semibold whitespace-nowrap text-ink',
                                        )}
                                    >
                                        {format.number(row.drops)}
                                    </td>
                                    <td className={cn(table.td, 'align-top')}>
                                        <div className="flex items-center gap-2.5 pt-1.5">
                                            <ShareBar
                                                share={row.share}
                                                first={!group.expected && index === 0}
                                            />
                                            <span className="w-10 text-right text-small">
                                                {formatShare(locale, row.share)}
                                            </span>
                                        </div>
                                    </td>
                                    <td className={cn(table.td, 'text-right align-top whitespace-nowrap')}>
                                        {rate(row, false)}
                                    </td>
                                    <td className={cn(table.td, 'align-top')}>
                                        {row.trend !== null && (
                                            <VerdictChip trend={row.trend} windows={windows} />
                                        )}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    ))}
                </table>
            </div>
            <div className="md:hidden">
                {ranked.map((group) => (
                    <Fragment key={String(group.expected)}>
                        <p className="border-b border-line bg-bg-subtle px-4 py-1.5 eyebrow">
                            {group.expected ? words.expected : words.unexpected}
                        </p>
                        <ul className="flex flex-col">
                            {group.rows.map((row, index) => (
                                <li
                                    key={row.category}
                                    className={cn(
                                        'flex flex-col gap-1.5 border-b border-line px-4 py-2.5',
                                        selected === row.category && SELECTED_ITEM,
                                    )}
                                >
                                    <div className="flex items-center justify-between gap-3">
                                        {name(row)}
                                        <span className="text-ui font-semibold whitespace-nowrap text-ink">
                                            {format.number(row.drops)}{' '}
                                            <span className="font-normal text-ink-3">
                                                {formatShare(locale, row.share)}
                                            </span>
                                        </span>
                                    </div>
                                    <ShareBar share={row.share} first={!group.expected && index === 0} />
                                    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-small text-ink-2">
                                        <span>{rate(row, true)}</span>
                                        {row.trend !== null && (
                                            <VerdictChip trend={row.trend} windows={windows} />
                                        )}
                                        {loading(row)}
                                    </div>
                                </li>
                            ))}
                        </ul>
                    </Fragment>
                ))}
            </div>
        </>
    );
}
