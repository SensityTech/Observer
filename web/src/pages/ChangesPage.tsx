import { keepPreviousData, useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, useNavigate, useSearch } from '@tanstack/react-router';
import { GitCompare, LoaderCircle } from 'lucide-react';
import { useState, type ReactNode } from 'react';

import { comparisonQuery, olderStartsQuery, startsQuery } from '../api/changes';
import { ApiError } from '../api/client';
import type { Comparison } from '../api/generated/Comparison';
import { useMe } from '../auth/me';
import { Notice } from '../components/Notice';
import { ComparisonBar } from '../changes/ComparisonBar';
import { EnvironmentCard } from '../changes/EnvironmentCard';
import { refreshMoved, useChangesLive } from '../changes/live';
import { ResourcesCard } from '../changes/ResourcesCard';
import type { ChangesSearch } from '../changes/search';
import { SettingsCard } from '../changes/SettingsCard';
import { StartsCard } from '../changes/StartsCard';
import { SummaryBand } from '../changes/SummaryBand';
import { isBusy } from '../changes/words';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

/** `/changes?from=&to=`, its search checked by the route. */
export function ChangesPage() {
    const { t } = useI18n();
    const { from, to } = useSearch({ strict: false });
    const search: ChangesSearch = { from, to };
    const me = useMe();
    const navigate = useNavigate();
    const queryClient = useQueryClient();
    // A snapshot still hashing settles resource by resource: what is shown follows it.
    const live = useChangesLive((moved) => refreshMoved(queryClient, moved));
    const starts = useQuery(startsQuery);
    // Older starts are read once asked for, from where the newest page stops.
    const [older, setOlder] = useState(false);
    const before = starts.data?.next ?? null;
    const olderStarts = useInfiniteQuery({
        ...olderStartsQuery(before ?? 0),
        enabled: older && before !== null,
        placeholderData: keepPreviousData,
    });
    const comparison = useQuery({
        ...comparisonQuery(search.from, search.to),
        placeholderData: keepPreviousData,
    });
    const compare = (next: ChangesSearch) => void navigate({ to: '/changes', search: next });

    const listed = [
        ...(starts.data?.starts ?? []),
        ...(older ? (olderStarts.data?.pages.flatMap((page) => page.starts) ?? []) : []),
    ];
    const check = live?.check ?? starts.data?.check ?? null;
    const compared = comparison.data;
    const missing = comparison.error instanceof ApiError && comparison.error.code === 'start_not_found';
    const defaults = search.from === undefined && search.to === undefined;

    let body: ReactNode;
    if (comparison.isPending || (missing && defaults && starts.isPending)) {
        body = (
            <div className="flex flex-col gap-5" aria-busy="true" aria-label={t.changes.loading}>
                <div className="skeleton h-28 w-full rounded-card" />
                <div className="skeleton h-64 w-full rounded-card" />
            </div>
        );
    } else if (missing && defaults && listed.length === 0) {
        body = <Empty title={t.changes.empty.title} body={t.changes.empty.body} />;
    } else if (missing && defaults) {
        body = <Empty title={t.changes.empty.baselineTitle} body={t.changes.empty.baselineBody} />;
    } else if (comparison.isError && compared === undefined) {
        body = (
            <Notice tone="danger">
                <p>{errorMessage(t, comparison.error)}</p>
                {!defaults && (
                    <Link
                        to="/changes"
                        className="mt-1 inline-block font-medium underline underline-offset-2"
                    >
                        {t.changes.empty.latest}
                    </Link>
                )}
            </Notice>
        );
    } else if (compared !== undefined) {
        body = <Compared comparison={compared} stale={comparison.isPlaceholderData} />;
    }

    return (
        <>
            <div className="page-head">
                <h1 className="page-title">
                    {t.changes.title} <span className="gloss">{t.changes.gloss}</span>
                </h1>
                <p className="text-meta text-ink-3">{t.changes.timeNote}</p>
            </div>
            {(compared !== undefined || listed.length > 0) && (
                <ComparisonBar
                    search={search}
                    compared={compared}
                    starts={listed}
                    check={check}
                    onCompare={compare}
                />
            )}
            {body}
            {listed.length > 0 && (
                <StartsCard
                    starts={listed}
                    current={search.to === 'disk' ? null : (compared?.to.id ?? null)}
                    more={before !== null && (!older || olderStarts.isPending || olderStarts.hasNextPage)}
                    loadingMore={older && (olderStarts.isPending || olderStarts.isFetchingNextPage)}
                    onLoadMore={() => (older ? void olderStarts.fetchNextPage() : setOlder(true))}
                    onPick={(start) => compare({ to: start.id })}
                />
            )}
            {me.isOwner && <SettingsCard />}
        </>
    );
}

/** The two snapshots compared: the facts, the environment and the resources. */
function Compared({ comparison, stale }: { comparison: Comparison; stale: boolean }) {
    const { t } = useI18n();
    const { to } = comparison;
    return (
        <div className={cn('flex flex-col gap-5 max-md:gap-4', stale && 'opacity-50')} aria-busy={stale}>
            {isBusy(to) && (
                <output className="flex items-center gap-2 text-small text-ink-2">
                    <LoaderCircle size={14} strokeWidth={1.5} className="animate-spin" aria-hidden="true" />
                    {t.changes.busy[to.state === 'listing' ? 'listing' : 'hashing']}
                </output>
            )}
            {to.state === 'partial' && to.partialReason !== null && (
                <Notice tone="warning">
                    {t.changes.partial(t.changes.partialReasons[to.partialReason])}
                </Notice>
            )}
            {to.state === 'failed' && <Notice tone="warning">{t.changes.failed}</Notice>}
            <SummaryBand comparison={comparison} />
            <EnvironmentCard comparison={comparison} />
            {/* Filters picked for one comparison mean nothing for another. */}
            <ResourcesCard key={`${comparison.from.id}:${comparison.to.id}`} comparison={comparison} />
        </div>
    );
}

function Empty({ title, body }: { title: string; body: string }) {
    return (
        <section className="flex flex-col items-center gap-3 rounded-card border border-dashed border-line-strong bg-bg-subtle px-6 py-14 text-center dot-grid">
            <span className="grid size-12 place-items-center rounded-box border border-line-strong bg-surface text-ink-2 shadow-sm">
                <GitCompare size={20} strokeWidth={1.5} aria-hidden="true" />
            </span>
            <h2 className="text-touch font-semibold text-ink">{title}</h2>
            <p className="max-w-empty text-body text-ink-3">{body}</p>
        </section>
    );
}
