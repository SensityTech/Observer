import { useNavigate, useSearch } from '@tanstack/react-router';
import { useMemo, useRef, type KeyboardEvent } from 'react';

import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { LiveMetricsProvider } from '../metrics/live';
import { InsightsTab } from '../statistics/InsightsTab';
import { parseStatisticsSearch, type StatisticsSearch, type Tab } from '../statistics/search';
import { TimelineTab } from '../statistics/TimelineTab';

const TABS: readonly Tab[] = ['timeline', 'insights'];

/**
 * Statistics: the timeline, every lane on one time axis with what happened along it, and the
 * insights drawn from weeks of it. The tab, ranges, zoom and lanes live in the address.
 */
export function StatisticsPage() {
    const { t } = useI18n();
    const navigate = useNavigate();
    const raw = useSearch({ strict: false });
    const search = useMemo(() => parseStatisticsSearch(raw as Record<string, unknown>), [raw]);
    const tab = search.tab ?? 'timeline';
    const tabs = useRef<(HTMLButtonElement | null)[]>([]);
    const go = (next: StatisticsSearch, replace = false) =>
        void navigate({ to: '/statistics', search: next, replace, resetScroll: false });
    const choose = (next: Tab) => go({ ...search, tab: next === 'timeline' ? undefined : next }, true);

    // Arrows, Home and End move between the tabs and open the one they reach.
    const keyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
        const at = TABS.indexOf(tab);
        const next = (
            {
                ArrowRight: (at + 1) % TABS.length,
                ArrowLeft: (at - 1 + TABS.length) % TABS.length,
                Home: 0,
                End: TABS.length - 1,
            } as Record<string, number | undefined>
        )[event.key];
        if (next === undefined) {
            return;
        }
        event.preventDefault();
        choose(TABS[next] ?? 'timeline');
        tabs.current[next]?.focus();
    };

    return (
        <LiveMetricsProvider>
            <div className="flex flex-col gap-4">
                <div className="page-head">
                    <h1 className="page-title">{t.statistics.title}</h1>
                </div>
                <div
                    role="tablist"
                    aria-label={t.statistics.tabs.label}
                    className="flex gap-0.5 border-b border-line"
                >
                    {TABS.map((each, index) => (
                        <button
                            key={each}
                            ref={(element) => {
                                tabs.current[index] = element;
                            }}
                            type="button"
                            role="tab"
                            id={`statistics-tab-${each}`}
                            aria-selected={tab === each}
                            aria-controls={`statistics-panel-${each}`}
                            tabIndex={tab === each ? 0 : -1}
                            onClick={() => choose(each)}
                            onKeyDown={keyDown}
                            className={cn(
                                'px-3.5 py-2.5 text-ui transition-colors max-md:h-11',
                                tab === each
                                    ? 'font-medium text-ink shadow-[inset_0_-2px_0_var(--ink)]'
                                    : 'text-ink-3 hover:text-ink',
                            )}
                        >
                            {t.statistics.tabs[each]}
                        </button>
                    ))}
                </div>
                <div role="tabpanel" id={`statistics-panel-${tab}`} aria-labelledby={`statistics-tab-${tab}`}>
                    {tab === 'timeline' ? (
                        <TimelineTab search={search} go={go} />
                    ) : (
                        <InsightsTab search={search} go={go} />
                    )}
                </div>
            </div>
        </LiveMetricsProvider>
    );
}
