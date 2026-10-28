import { useQuery } from '@tanstack/react-query';
import { Layers } from 'lucide-react';
import { useId } from 'react';

import type { ThreadShares } from '../api/generated/ThreadShares';
import { threadsQuery } from '../api/metrics';
import { table } from '../components/table';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

/** The busiest names the card lists; the rest go into one row. */
const SHOWN = 8;
/** Under this share of one core, in percent, the rest is not worth a row. */
const OTHERS_FROM = 0.05;

export type ThreadRow = { name: string | null; cpu: number };

/** The rows of the card: the busiest names, then the rest of the process's CPU as `null`. */
export const threadRows = (shares: ThreadShares): ThreadRow[] => {
    const shown: ThreadRow[] = shares.threads.slice(0, SHOWN).map(({ name, cpu }) => ({ name, cpu }));
    const listed = shown.reduce((sum, row) => sum + row.cpu, 0);
    const rest = (shares.process ?? 0) - listed;
    return rest > OTHERS_FROM ? [...shown, { name: null, cpu: rest }] : shown;
};

/** The main process's threads over the last hour, by CPU. */
export function ThreadsCard() {
    const { t, format } = useI18n();
    const titleId = useId();
    const shares = useQuery(threadsQuery);
    const rows = shares.data ? threadRows(shares.data) : [];
    const process = shares.data?.process ?? null;
    // The bar is the row's part of the whole process, in percent.
    const shareOf = (row: ThreadRow) =>
        process === null || process <= 0 ? 0 : Math.min(100, (row.cpu / process) * 100);

    let body;
    if (shares.isPending) {
        body = <div className="skeleton m-5 h-24" aria-busy="true" aria-label={t.threads.loading} />;
    } else if (rows.length === 0) {
        body = (
            <p className="px-5 py-4 text-ui text-ink-3 max-md:px-4">
                {shares.isError ? t.threads.failed : t.threads.empty}
            </p>
        );
    } else {
        body = (
            <div className={table.scroll}>
                <table className={table.table}>
                    <caption className="sr-only">{t.threads.caption}</caption>
                    <thead>
                        <tr>
                            <th scope="col" className={table.th}>
                                {t.threads.thread}
                            </th>
                            <th scope="col" className={cn(table.th, 'w-[34%]')}>
                                <span className="sr-only">{t.threads.share}</span>
                            </th>
                            <th scope="col" className={cn(table.th, 'text-right')}>
                                {t.threads.cpu}
                            </th>
                        </tr>
                    </thead>
                    <tbody>
                        {rows.map((row, index) => (
                            <tr key={row.name ?? ''} className={table.row}>
                                <td className={table.td}>
                                    {row.name === null ? (
                                        <span className="text-ink-3">{t.threads.others}</span>
                                    ) : (
                                        <span
                                            className={cn(
                                                table.mono,
                                                'whitespace-normal [overflow-wrap:anywhere]',
                                            )}
                                        >
                                            {row.name === '' ? t.threads.unnamed : row.name}
                                        </span>
                                    )}
                                </td>
                                <td className={table.td}>
                                    <span className="sr-only">{format.percent(shareOf(row))}</span>
                                    <div className="h-1 rounded-[2px] bg-surface-3" aria-hidden="true">
                                        <span
                                            className={cn(
                                                'block h-full rounded-[2px]',
                                                index === 0 ? 'bg-accent' : 'bg-chart-muted',
                                            )}
                                            style={{ width: `${shareOf(row)}%` }}
                                        />
                                    </div>
                                </td>
                                <td
                                    className={cn(
                                        table.td,
                                        'w-[1%] text-right font-semibold whitespace-nowrap text-ink',
                                    )}
                                >
                                    {format.percent(row.cpu)}
                                </td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            </div>
        );
    }

    return (
        <section className="card" aria-labelledby={titleId}>
            <div className="card-head">
                <span
                    className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2"
                    aria-hidden="true"
                >
                    <Layers size={14} strokeWidth={1.75} />
                </span>
                <h2 id={titleId} className="card-title">
                    {t.threads.title}
                </h2>
                <span className="ml-auto text-meta text-ink-3">{t.threads.meta}</span>
            </div>
            {body}
        </section>
    );
}
