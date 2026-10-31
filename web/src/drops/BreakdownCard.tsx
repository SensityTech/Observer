import { ChartBar } from 'lucide-react';
import { Fragment } from 'react';

import type { CategoryShare } from '../api/generated/CategoryShare';
import type { DropCategory } from '../api/generated/DropCategory';
import type { DropsSummary } from '../api/generated/DropsSummary';
import { Card } from '../components/Card';
import { table } from '../components/table';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { CategoryWord, formatRate, formatShare, ShareBar, trendWindows, VerdictChip } from './words';

/** The selected row: the accent's soft fill and its bar against the first cell. */
const SELECTED = 'bg-accent-soft [&>td:first-child]:shadow-[inset_2px_0_0_var(--accent)]';

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
 * unexpected ones apart from the expected ones. A row narrows the chart, and the log when there is one.
 */
export function BreakdownCard({
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
    const toggle = (category: DropCategory) => onSelect(selected === category ? undefined : category);
    // The table's column names the unit; a phone's line says it.
    const rate = (row: CategoryShare, unit: boolean) => {
        if (row.rate === null) {
            return <span className="text-ink-3">{words.notPlayers}</span>;
        }
        const figure = formatRate(locale, row.rate);
        return unit ? t.drops.rate(figure) : figure;
    };
    const name = (row: CategoryShare) => (
        <button
            type="button"
            aria-pressed={selected === row.category}
            onClick={() => toggle(row.category)}
            className="-mx-1 rounded-xs px-1 text-left hover:underline max-md:min-h-11"
        >
            <CategoryWord category={row.category} />
        </button>
    );
    return (
        <Card
            id="drops-breakdown"
            icon={ChartBar}
            title={words.title}
            meta={words.meta(summary.sessions, format.playTime(summary.playedMs))}
            flush
        >
            {ranked.length === 0 || summary.categories.every((row) => row.drops === 0) ? (
                <p className="px-6 py-4 text-center text-ui text-ink-3">{words.empty}</p>
            ) : (
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
                                    <th scope="col" className={cn(table.th, 'w-[34%]')}>
                                        <span className="sr-only">{words.share}</span>
                                    </th>
                                    <th scope="col" className={cn(table.th, 'text-right')}>
                                        {words.rate}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {words.verdict}
                                    </th>
                                    <th scope="col" className={cn(table.th, 'text-right')}>
                                        {words.loading}
                                    </th>
                                </tr>
                            </thead>
                            {ranked.map((group) => (
                                <tbody key={String(group.expected)}>
                                    <tr>
                                        <th
                                            scope="rowgroup"
                                            colSpan={6}
                                            className="h-8 border-b border-line bg-bg-subtle px-5 text-left eyebrow max-md:px-4"
                                        >
                                            {group.expected ? words.expected : words.unexpected}
                                        </th>
                                    </tr>
                                    {group.rows.map((row, index) => (
                                        <tr
                                            key={row.category}
                                            className={
                                                selected === row.category ? SELECTED : 'hover:bg-surface-2'
                                            }
                                        >
                                            <td className={table.td}>{name(row)}</td>
                                            <td
                                                className={cn(
                                                    table.td,
                                                    'w-[1%] text-right font-semibold whitespace-nowrap text-ink',
                                                )}
                                            >
                                                {format.number(row.drops)}
                                            </td>
                                            <td className={table.td}>
                                                <div className="flex items-center gap-2.5">
                                                    <ShareBar
                                                        share={row.share}
                                                        first={!group.expected && index === 0}
                                                    />
                                                    <span className="w-10 text-right text-small">
                                                        {formatShare(locale, row.share)}
                                                    </span>
                                                </div>
                                            </td>
                                            <td className={cn(table.td, 'text-right whitespace-nowrap')}>
                                                {rate(row, false)}
                                            </td>
                                            <td className={table.td}>
                                                {row.trend !== null && (
                                                    <VerdictChip trend={row.trend} windows={windows} />
                                                )}
                                            </td>
                                            <td className={cn(table.td, 'text-right whitespace-nowrap')}>
                                                {row.loadingShare === null
                                                    ? ''
                                                    : formatShare(locale, row.loadingShare)}
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
                                                selected === row.category && SELECTED,
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
                                            <ShareBar
                                                share={row.share}
                                                first={!group.expected && index === 0}
                                            />
                                            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-small text-ink-2">
                                                <span>{rate(row, true)}</span>
                                                {row.trend !== null && (
                                                    <VerdictChip trend={row.trend} windows={windows} />
                                                )}
                                                {row.loadingShare !== null && (
                                                    <span className="text-ink-3">
                                                        {words.loadingShare(
                                                            formatShare(locale, row.loadingShare),
                                                        )}
                                                    </span>
                                                )}
                                            </div>
                                        </li>
                                    ))}
                                </ul>
                            </Fragment>
                        ))}
                    </div>
                </>
            )}
            <p className="rounded-b-card bg-bg-subtle px-5 py-2.5 text-meta text-ink-3 max-md:px-4">
                {withLog ? words.filterHint : words.filterHintChart}
            </p>
        </Card>
    );
}
