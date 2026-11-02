import { Link, useNavigate, useSearch } from '@tanstack/react-router';
import { useMemo } from 'react';

import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { LiveMetricsProvider } from '../metrics/live';
import { InsightsTab } from '../statistics/InsightsTab';
import { parseStatisticsSearch, type StatisticsSearch } from '../statistics/search';
import { TimelineTab } from '../statistics/TimelineTab';

const VIEW = 'px-3.5 py-2.5 text-ui transition-colors max-md:inline-flex max-md:h-11 max-md:items-center';
const CURRENT = 'font-medium text-ink shadow-[inset_0_-2px_0_var(--ink)]';
const OTHER = 'text-ink-3 hover:text-ink';
/** A view is the current page only for its own address: the timeline is not current on the insights. */
const EXACT = { exact: true, includeSearch: true } as const;

/**
 * Statistics: the timeline, every lane on one time axis with what happened along it, and the
 * insights drawn from weeks of it. The view, ranges, zoom and lanes live in the address; the players
 * and stability views are the Players and Drops pages, for who may open them.
 */
export function StatisticsPage() {
    const { t } = useI18n();
    const me = useMe();
    const navigate = useNavigate();
    const raw = useSearch({ strict: false });
    const search = useMemo(() => parseStatisticsSearch(raw as Record<string, unknown>), [raw]);
    const tab = search.tab ?? 'timeline';
    const go = (next: StatisticsSearch, replace = false) =>
        void navigate({ to: '/statistics', search: next, replace, resetScroll: false });

    return (
        <LiveMetricsProvider>
            <div className="flex flex-col gap-4">
                <div className="page-head">
                    <h1 className="page-title">{t.statistics.title}</h1>
                </div>
                <nav
                    aria-label={t.statistics.tabs.label}
                    className="flex flex-wrap gap-0.5 border-b border-line"
                >
                    <Link
                        to="/statistics"
                        search={{ ...search, tab: undefined }}
                        replace
                        activeOptions={EXACT}
                        className={cn(VIEW, tab === 'timeline' ? CURRENT : OTHER)}
                    >
                        {t.statistics.tabs.timeline}
                    </Link>
                    <Link
                        to="/statistics"
                        search={{ ...search, tab: 'insights' }}
                        replace
                        activeOptions={EXACT}
                        className={cn(VIEW, tab === 'insights' ? CURRENT : OTHER)}
                    >
                        {t.statistics.tabs.insights}
                    </Link>
                    {allows(me, Permission.PlayersView) && (
                        <Link to="/players" className={cn(VIEW, OTHER)}>
                            {t.statistics.tabs.players}
                        </Link>
                    )}
                    {allows(me, Permission.DropsView) && (
                        <Link to="/drops" className={cn(VIEW, OTHER)}>
                            {t.statistics.tabs.stability}
                        </Link>
                    )}
                </nav>
                {tab === 'timeline' ? (
                    <TimelineTab search={search} go={go} />
                ) : (
                    <InsightsTab search={search} go={go} />
                )}
            </div>
        </LiveMetricsProvider>
    );
}
