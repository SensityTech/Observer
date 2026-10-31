import { ListFilter, UserX } from 'lucide-react';
import { useState, type ReactNode } from 'react';

import type { ReasonGroup } from '../api/generated/ReasonGroup';
import type { ResourceEntry } from '../api/generated/ResourceEntry';
import type { ResourceTable } from '../api/generated/ResourceTable';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { formatShare, keptSince, ShareBar, VerdictChip } from './words';

const number = cn(table.td, 'w-[1%] text-right font-semibold whitespace-nowrap text-ink');

/** A row's key, which kicks no resource is named for share. */
const key = (entry: ResourceEntry) => entry.resource ?? '';

/**
 * The resources that kicked players, ranked as the signatures are, each row's actions under its
 * name. What a resource gave as its reasons only reaches a viewer who sees players: the API leaves
 * them out for the others.
 */
export function ResourcesCard({
    resources,
    failed,
    stale,
    windows,
    rangeFrom,
    zone,
    onShowInLog,
}: {
    resources: ResourceTable | undefined;
    failed: boolean;
    /** Still the previous range's, while the new one is read. */
    stale: boolean;
    windows: string;
    /** The range's start, to tell when players and reasons come from a shorter stretch. */
    rangeFrom: string | undefined;
    zone: string;
    /** Narrows the log to a resource; only for a viewer who sees the log. */
    onShowInLog?: (resource: string) => void;
}) {
    const { t, locale, format } = useI18n();
    const words = t.drops.resources;
    const [open, setOpen] = useState<string | null>(null);
    const since = resources === undefined ? null : keptSince(locale, resources.keptFrom, rangeFrom, zone);

    let body: ReactNode;
    if (resources === undefined) {
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
                <span className="skeleton w-1/2" />
            </div>
        );
    } else if (resources.resources.length === 0) {
        body = <p className="px-6 py-4 text-center text-ui text-ink-3">{words.empty}</p>;
    } else {
        const ranked = resources.resources.toSorted((a, b) => b.kicks - a.kicks);
        const actions = (entry: ResourceEntry) => {
            const resource = entry.resource;
            const reasons = entry.reasons !== null;
            const log = onShowInLog !== undefined && resource !== null;
            if (!reasons && !log) {
                return null;
            }
            return (
                <div className="-ml-2.5 flex flex-wrap gap-1 max-md:-ml-0 max-md:gap-2">
                    {reasons && (
                        <Button
                            size="sm"
                            tone="ghost"
                            className="h-7"
                            aria-expanded={open === key(entry)}
                            onClick={() => setOpen(open === key(entry) ? null : key(entry))}
                        >
                            {open === key(entry) ? words.hideReasons : words.reasons}
                        </Button>
                    )}
                    {log && (
                        <Button
                            size="sm"
                            tone="ghost"
                            icon={ListFilter}
                            className="h-7"
                            onClick={() => onShowInLog(resource)}
                        >
                            {words.showInLog}
                        </Button>
                    )}
                </div>
            );
        };
        const name = (entry: ResourceEntry, withActions: boolean) => (
            <div className="flex min-w-0 flex-col gap-0.75">
                {entry.resource === null ? (
                    <span className="text-ink-3">{words.noResource}</span>
                ) : (
                    <span className={cn(table.mono, 'whitespace-normal [overflow-wrap:anywhere]')}>
                        {entry.resource}
                    </span>
                )}
                {withActions && actions(entry)}
            </div>
        );
        body = (
            <>
                <div className={cn(table.scroll, 'rounded-b-card max-md:hidden')}>
                    <table className={table.table}>
                        <caption className="sr-only">{words.caption}</caption>
                        <thead>
                            <tr>
                                <th scope="col" className={table.th}>
                                    {words.resource}
                                </th>
                                <th scope="col" className={cn(table.th, 'text-right')}>
                                    {words.kicks}
                                </th>
                                <th scope="col" className={cn(table.th, 'w-[24%]')}>
                                    <span className="sr-only">{words.share}</span>
                                </th>
                                <th scope="col" className={cn(table.th, 'text-right')}>
                                    {words.players}
                                </th>
                                <th scope="col" className={table.th}>
                                    {words.verdict}
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            {ranked.map((entry, index) => (
                                <ResourceRows key={key(entry)} entry={entry} open={open === key(entry)}>
                                    <td className={cn(table.td, 'min-w-40 align-top')}>
                                        {name(entry, true)}
                                    </td>
                                    <td className={cn(number, 'align-top')}>{format.number(entry.kicks)}</td>
                                    <td className={cn(table.td, 'align-top')}>
                                        <div className="flex items-center gap-2.5 pt-1.5">
                                            <ShareBar share={entry.share} first={index === 0} />
                                            <span className="w-10 text-right text-small">
                                                {formatShare(locale, entry.share)}
                                            </span>
                                        </div>
                                    </td>
                                    <td className={cn(table.td, 'text-right align-top')}>
                                        {format.number(entry.players)}
                                    </td>
                                    <td className={cn(table.td, 'align-top')}>
                                        <VerdictChip trend={entry.trend} windows={windows} />
                                    </td>
                                </ResourceRows>
                            ))}
                        </tbody>
                    </table>
                </div>
                <ul className="flex flex-col md:hidden">
                    {ranked.map((entry, index) => (
                        <li
                            key={key(entry)}
                            className="flex flex-col gap-1.5 border-b border-line px-4 py-3 last:border-b-0"
                        >
                            <div className="flex items-baseline justify-between gap-3">
                                {name(entry, false)}
                                <span className="text-small whitespace-nowrap text-ink">
                                    {words.meta(entry.kicks)} · {formatShare(locale, entry.share)}
                                </span>
                            </div>
                            <ShareBar share={entry.share} first={index === 0} />
                            <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-small text-ink-2">
                                <span>{words.playersCount(entry.players)}</span>
                                <VerdictChip trend={entry.trend} windows={windows} />
                            </div>
                            {actions(entry)}
                            {open === key(entry) && entry.reasons !== null && (
                                <Reasons resource={entry.resource} reasons={entry.reasons} />
                            )}
                        </li>
                    ))}
                </ul>
            </>
        );
    }
    return (
        <Card
            id="drops-resources"
            icon={UserX}
            title={words.title}
            meta={
                resources === undefined
                    ? undefined
                    : [words.meta(resources.kicks), since === null ? null : t.drops.playersSince(since)]
                          .filter((part) => part !== null)
                          .join(' · ')
            }
            flush
        >
            <div aria-busy={stale} className={cn('transition-opacity', stale && 'opacity-50')}>
                {body}
            </div>
        </Card>
    );
}

function ResourceRows({
    entry,
    open,
    children,
}: {
    entry: ResourceEntry;
    open: boolean;
    children: ReactNode;
}) {
    return (
        <>
            <tr className={cn(table.row, open && '[&>td]:border-b-0')}>{children}</tr>
            {open && entry.reasons !== null && (
                <tr>
                    <td colSpan={5} className={cn(table.td, 'bg-bg-subtle')}>
                        <Reasons resource={entry.resource} reasons={entry.reasons} />
                    </td>
                </tr>
            )}
        </>
    );
}

/** What a resource gave as its reasons, grouped once numbers and names were taken out: plain text. */
function Reasons({ resource, reasons }: { resource: string | null; reasons: ReasonGroup[] }) {
    const { t } = useI18n();
    const words = t.drops.resources;
    if (reasons.length === 0) {
        return <p className="py-2 text-small text-ink-3">{words.noReason}</p>;
    }
    return (
        <div className="flex flex-col gap-2 py-2">
            <ul
                className="flex flex-col gap-1"
                aria-label={words.reasonsCaption(resource ?? words.noResource)}
            >
                {reasons
                    .toSorted((a, b) => b.kicks - a.kicks)
                    .map((reason) => (
                        <li key={reason.template} className="flex items-baseline justify-between gap-3">
                            <span className="font-mono text-small whitespace-pre-wrap text-ink [font-variant-ligatures:none] [overflow-wrap:anywhere]">
                                {reason.template}
                            </span>
                            <span className="text-small whitespace-nowrap text-ink-2">
                                {words.reasonKicks(reason.kicks)}
                            </span>
                        </li>
                    ))}
            </ul>
            <p className="text-meta text-ink-3">{words.reasonsNote}</p>
        </div>
    );
}
