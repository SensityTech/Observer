import { keepPreviousData, useMutation, useQuery } from '@tanstack/react-query';
import { useNavigate, useSearch } from '@tanstack/react-router';
import { Download } from 'lucide-react';
import { useMemo, useRef } from 'react';

import {
    changesQuery,
    downloadRates,
    resourcesQuery,
    signaturesQuery,
    summaryQuery,
    uptimeQuery,
} from '../api/drops';
import type { ChartCategory } from '../api/generated/ChartCategory';
import type { DropCategory } from '../api/generated/DropCategory';
import type { DropsSummary } from '../api/generated/DropsSummary';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Notice } from '../components/Notice';
import { BreakdownCard } from '../drops/BreakdownCard';
import { ChangesCard } from '../drops/ChangesCard';
import { DropsChart } from '../drops/DropsChart';
import { DropTiles } from '../drops/DropTiles';
import { LogCard } from '../drops/LogCard';
import { ResourcesCard } from '../drops/ResourcesCard';
import {
    chartShown,
    DEFAULT_RANGE,
    parseDropsSearch,
    RANGES,
    withCategory,
    withFilters,
    type DropsSearch,
} from '../drops/search';
import { SettingsCard } from '../drops/SettingsCard';
import { SignaturesCard } from '../drops/SignaturesCard';
import { UptimeCard } from '../drops/UptimeCard';
import { formatShare, keptSince, trendWindows } from '../drops/words';
import { errorMessage } from '../i18n/errors';
import { zoneDate } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { ZoneNote } from '../players/words';
import { useSchedule } from '../schedule/context';

/** Repeat crashers are counted over the last 7 days, whatever the range. */
const REPEAT_DAYS = 7;

/** Above this share of sessions Observer had to close itself, the page says how many. */
const COVERAGE_GAP = 0.05;

/** The range's rate of the drops the chart shows, as the summary has it. */
const chartRate = (summary: DropsSummary | undefined, category: ChartCategory) => {
    if (summary === undefined) {
        return null;
    }
    if (category === 'security') {
        return summary.categories.find((each) => each.category === 'security')?.rate ?? null;
    }
    const kind = (
        { unexpected: 'unexpected', crash: 'crashes', timeout: 'timeouts', kick: 'kicks' } as const
    )[category];
    return summary.tiles.find((tile) => tile.kind === kind)?.rate ?? null;
};

/**
 * How players' sessions end: rates per 100 player-hours and whether they changed, the chart, the
 * categories, crash signatures, kicking resources, what changed on the server, and the log itself for
 * who sees players. The range, the chart and the log's filters live in the address.
 */
export function DropsPage() {
    const { t, locale } = useI18n();
    const me = useMe();
    const navigate = useNavigate();
    const raw = useSearch({ strict: false });
    const search = useMemo(() => parseDropsSearch(raw as Record<string, unknown>), [raw]);
    const range = search.range ?? DEFAULT_RANGE;
    const withLog = allows(me, Permission.PlayersView);
    const logTitle = useRef<HTMLHeadingElement>(null);

    const summary = useQuery({ ...summaryQuery(range), placeholderData: keepPreviousData });
    const signatures = useQuery({ ...signaturesQuery(range), placeholderData: keepPreviousData });
    const resources = useQuery({ ...resourcesQuery(range), placeholderData: keepPreviousData });
    const changes = useQuery({ ...changesQuery(range), placeholderData: keepPreviousData });
    const uptime = useQuery({ ...uptimeQuery(range), placeholderData: keepPreviousData });
    const scheduled = useSchedule().zone;
    const zone = summary.data?.zone ?? scheduled ?? 'UTC';
    const windows =
        summary.data === undefined
            ? t.drops.windows[range === '24h' ? 'day' : range]
            : trendWindows(t, locale, summary.data);

    const go = (next: DropsSearch, replace: boolean) =>
        navigate({ to: '/drops', search: next, replace, resetScroll: false });
    // A card's link narrows the log, then takes the keyboard there, which scrolls to it.
    const showInLog = (filters: Omit<DropsSearch, 'range' | 'chart'>) =>
        void go(withFilters(search, filters), false).then(() => logTitle.current?.focus());
    const select = (category: DropCategory | undefined) => void go(withCategory(search, category), false);

    const rates = useMutation({
        mutationFn: (current: DropsSummary) =>
            downloadRates(zoneDate(current.from, current.zone), zoneDate(current.to, current.zone)),
    });

    const coverage = summary.data?.coverage;
    const closed = coverage === undefined ? 0 : coverage.missed + coverage.lost;
    const stale = summary.isPlaceholderData;

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.drops.title} <span className="gloss">{t.drops.gloss}</span>
                </h1>
                <div className="flex flex-wrap items-center gap-3 max-md:w-full">
                    <ZoneNote zone={zone} />
                    <fieldset className="inline-flex max-w-full gap-0.5 rounded-control border border-line bg-surface-2 p-0.75">
                        <legend className="sr-only">{t.drops.range}</legend>
                        {RANGES.map((choice) => (
                            <button
                                key={choice}
                                type="button"
                                aria-pressed={range === choice}
                                onClick={() =>
                                    void go(
                                        { ...search, range: choice === DEFAULT_RANGE ? undefined : choice },
                                        true,
                                    )
                                }
                                className="inline-flex h-6.5 items-center rounded-[7px] px-2.5 text-small font-medium text-ink-3 transition-colors hover:text-ink focus-visible:outline-offset-1 aria-pressed:bg-raise aria-pressed:text-ink aria-pressed:shadow-[var(--elev-sm),0_0_0_1px_var(--line)] max-md:h-11 max-md:min-w-11 max-md:justify-center"
                            >
                                {t.drops.ranges[choice]}
                            </button>
                        ))}
                    </fieldset>
                    <Button
                        icon={Download}
                        pending={rates.isPending}
                        disabled={summary.data === undefined}
                        className="max-md:flex-1"
                        onClick={() => summary.data !== undefined && rates.mutate(summary.data)}
                    >
                        {t.drops.exportRates}
                    </Button>
                </div>
            </div>
            {rates.isError && (
                <Notice tone="danger">
                    {t.drops.exportFailed} {errorMessage(t, rates.error)}
                </Notice>
            )}
            {summary.isError && summary.data === undefined && (
                <Notice tone="danger">
                    {t.drops.failed} {errorMessage(t, summary.error)}
                </Notice>
            )}
            {coverage !== undefined && coverage.sessions > 0 && closed / coverage.sessions > COVERAGE_GAP && (
                <Notice>
                    {t.drops.coverage(
                        closed,
                        coverage.sessions,
                        formatShare(locale, closed / coverage.sessions),
                        coverage.missed,
                        coverage.lost,
                    )}
                </Notice>
            )}
            <div aria-busy={summary.isFetching} className={cn('transition-opacity', stale && 'opacity-50')}>
                {summary.data === undefined ? (
                    summary.isPending && (
                        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4" aria-label={t.drops.loading}>
                            {[0, 1, 2, 3].map((index) => (
                                <div key={index} className="skeleton h-40 w-full rounded-card" />
                            ))}
                        </div>
                    )
                ) : (
                    <DropTiles summary={summary.data} />
                )}
            </div>
            <DropsChart
                range={range}
                category={chartShown(search)}
                onCategory={(chart) => void go({ ...search, chart }, true)}
                rate={chartRate(summary.data, chartShown(search))}
                zone={zone}
            />
            <div className="grid grid-cols-1 gap-3 2xl:grid-cols-3">
                <div className="flex min-w-0 flex-col 2xl:col-span-2 [&>section]:flex-1">
                    {summary.data !== undefined && (
                        <BreakdownCard
                            summary={summary.data}
                            selected={search.category}
                            onSelect={select}
                            withLog={withLog}
                        />
                    )}
                </div>
                <div className="flex min-w-0 flex-col [&>section]:flex-1">
                    <UptimeCard
                        uptime={uptime.data}
                        failed={uptime.isError}
                        rangeFrom={summary.data?.from}
                        zone={zone}
                    />
                </div>
            </div>
            <SignaturesCard
                signatures={signatures.data}
                failed={signatures.isError}
                zone={zone}
                windows={windows}
                rangeFrom={summary.data?.from}
                repeat={summary.data?.repeatCrashers ?? []}
                repeatSince={
                    summary.data === undefined
                        ? null
                        : keptSince(
                              locale,
                              summary.data.keptFrom,
                              new Date(Date.parse(summary.data.to) - REPEAT_DAYS * 86_400_000).toISOString(),
                              zone,
                          )
                }
                seesPlayers={withLog}
                onShowInLog={withLog ? (signature) => showInLog({ signature }) : undefined}
            />
            <div className="grid grid-cols-1 gap-3 2xl:grid-cols-3">
                <div className="flex min-w-0 flex-col 2xl:col-span-2 [&>section]:flex-1">
                    <ResourcesCard
                        resources={resources.data}
                        failed={resources.isError}
                        windows={windows}
                        rangeFrom={summary.data?.from}
                        zone={zone}
                        onShowInLog={withLog ? (resource) => showInLog({ resource }) : undefined}
                    />
                </div>
                <div className="flex min-w-0 flex-col [&>section]:flex-1">
                    <ChangesCard
                        changes={changes.data}
                        failed={changes.isError}
                        zone={zone}
                        onShowRun={withLog ? (run) => showInLog({ run }) : undefined}
                    />
                </div>
            </div>
            {withLog && (
                <LogCard
                    search={{ ...search, range }}
                    signatures={signatures.data}
                    zone={zone}
                    titleRef={logTitle}
                    onFilters={(filters) => void go(withFilters(search, filters), false)}
                />
            )}
            {me.isOwner && <SettingsCard />}
        </div>
    );
}
