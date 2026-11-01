import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, RotateCw } from 'lucide-react';
import { Suspense, useId } from 'react';

import { Permission } from '../api/generated/Permission';
import type { ScheduleStatus } from '../api/generated/ScheduleStatus';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { launchQuery } from '../api/launch';
import { useArtifacts } from '../artifacts/context';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { formatClock, formatDay, zoneDate } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import { cn } from '../lib/cn';
import { lazyCode } from '../lib/lazyCode';
import { secondsSince, useNow } from '../lib/useNow';
import { LiveMetricsProvider, useLiveMetrics } from '../metrics/live';
import { Alerts } from '../overview/Alerts';
import { ChangesFact } from '../overview/ChangesFact';
import { ThreadsCard } from '../overview/ThreadsCard';
import { Tiles } from '../overview/Tiles';
import { TreeCard } from '../overview/TreeCard';
import { useSchedule } from '../schedule/context';
import { useServerStatus } from '../server/context';
import { HangAdvice } from '../server/HangAdvice';
import { STATE_INK, STATE_LED, buildFact, causeText, isRunning } from '../server/state';

// The chart library is the heaviest part of the panel: it loads after the page has painted.
const PlayersChart = lazyCode(() => import('../charts/PlayersChart'));
const TickChart = lazyCode(() => import('../charts/TickChart'));

export function OverviewPage() {
    const { t } = useI18n();

    return (
        <>
            <div className="page-head">
                <h1 className="page-title">
                    {t.overview.title} <span className="gloss">{t.overview.gloss}</span>
                </h1>
            </div>
            <LiveMetricsProvider>
                <ServerCard />
                <Alerts />
                <Tiles />
                <ErrorBoundary
                    fallback={
                        <ChartFailed title={t.charts.players.title} text={t.charts.players.unavailable} />
                    }
                >
                    <Suspense
                        fallback={<ChartPlaceholder label={t.charts.players.loading} height="h-[260px]" />}
                    >
                        <PlayersChart />
                    </Suspense>
                </ErrorBoundary>
                <div className="grid grid-cols-1 gap-3 2xl:grid-cols-3">
                    <div className="flex min-w-0 flex-col 2xl:col-span-2">
                        <ErrorBoundary
                            fallback={
                                <ChartFailed title={t.charts.tick.title} text={t.charts.tick.unavailable} />
                            }
                        >
                            <Suspense
                                fallback={
                                    <ChartPlaceholder label={t.charts.tick.loading} height="h-[240px]" />
                                }
                            >
                                <TickChart />
                            </Suspense>
                        </ErrorBoundary>
                    </div>
                    <div className="flex min-w-0 flex-col gap-3">
                        <ThreadsCard />
                        <TreeCard />
                    </div>
                </div>
            </LiveMetricsProvider>
        </>
    );
}

/** A chart card's shape while its code loads. */
function ChartPlaceholder({ label, height }: { label: string; height: string }) {
    return (
        <section className="card flex-1" aria-busy="true" aria-label={label}>
            <div className="card-head">
                <span className="skeleton h-3 w-40" />
            </div>
            <div className="card-body">
                <div className={`skeleton ${height} w-full max-md:h-[200px]`} />
            </div>
        </section>
    );
}

/** A chart card when its code could not be loaded, or it could not be drawn: the rest stays. */
function ChartFailed({ title, text }: { title: string; text: string }) {
    const { t } = useI18n();
    const titleId = useId();
    return (
        <section className="card flex-1" aria-labelledby={titleId}>
            <div className="card-head">
                <h2 id={titleId} className="card-title">
                    {title}
                </h2>
            </div>
            <div className="card-body flex flex-col items-start gap-3">
                <p className="text-ui text-ink-3">{text}</p>
                <Button icon={RotateCw} onClick={() => window.location.reload()}>
                    {t.common.reload}
                </Button>
            </div>
        </section>
    );
}

/** FXServer as the supervisor sees it: the state in a word and why, then the facts that go with it. */
function ServerCard() {
    const { t } = useI18n();
    const link = useLinkState();
    const { status, refused } = useServerStatus();

    let body;
    if (refused) {
        body = <p className="text-ink-3">{t.overview.serverForbidden}</p>;
    } else if (status === null) {
        body = (
            <p className="text-ink-3" aria-busy={link === 'live' || link === 'connecting'}>
                {link === 'live' || link === 'connecting'
                    ? t.overview.serverLoading
                    : t.overview.serverUnknown}
            </p>
        );
    } else {
        body = <ServerState status={status} />;
    }

    return (
        <section className="card" aria-labelledby="overview-server">
            <div className="card-head">
                <h2 id="overview-server" className="eyebrow">
                    {t.overview.stateTitle}
                </h2>
            </div>
            <div className="card-body flex flex-col gap-3">{body}</div>
        </section>
    );
}

/** The next restart in the schedule's zone, which phones, without the status bar's cell, read here. */
const nextRestart = (
    t: ReturnType<typeof useI18n>['t'],
    locale: string,
    format: ReturnType<typeof useI18n>['format'],
    schedule: ScheduleStatus | null,
    now: number,
) => {
    const next = schedule?.next;
    if (schedule === null || next === undefined || next === null) {
        return null;
    }
    const clock = formatClock(locale, next.at, schedule.zone);
    const today = zoneDate(next.at, schedule.zone) === zoneDate(now, schedule.zone);
    return t.overview.nextRestart(
        today
            ? clock
            : t.schedule.dayTime(formatDay(locale, next.at, schedule.zone, { weekday: false }), clock),
        format.duration(Math.max(0, Date.parse(next.at) - now) / 1000),
    );
};

function ServerState({ status }: { status: ServerStatus }) {
    const { t, locale, format } = useI18n();
    const me = useMe();
    const now = useNow();
    const { latest } = useLiveMetrics();
    const { schedule, zone } = useSchedule();
    const { status: builds } = useArtifacts();
    // Only the owner reads the launch settings, and can change the delay a hang names.
    const launchSettings = useQuery({ ...launchQuery, enabled: me.isOwner });
    const cause = causeText(t, status);
    const running = isRunning(status.state);
    const resources = running ? (latest?.values.resources ?? null) : null;

    const facts = [
        running && status.startedAt !== null
            ? t.overview.started(
                  format.timestamp(status.startedAt, { seconds: false }),
                  format.duration(secondsSince(status.startedAt, now)),
              )
            : null,
        buildFact(t, status, builds, (instant) =>
            zone === null
                ? t.overview.utcClock(formatClock(locale, instant, 'UTC'))
                : formatClock(locale, instant, zone),
        ) ?? (status.build === null ? null : t.overview.build(String(status.build))),
        status.players === null
            ? null
            : t.overview.players(
                  t.status.playerCount(
                      format.number(status.players.online),
                      status.players.max === null ? null : format.number(status.players.max),
                  ),
              ),
        resources === null ? null : t.overview.resources(resources),
        status.restartAt === null
            ? null
            : t.server_state.restartIn(
                  format.duration(Math.max(0, Date.parse(status.restartAt) - now) / 1000),
              ),
        running ? nextRestart(t, locale, format, schedule, now) : null,
    ].filter((fact) => fact !== null);

    const launch =
        status.launch === 'ready'
            ? null
            : status.launch === 'notConfigured'
              ? [
                    t.overview.notConfigured,
                    me.isOwner ? t.overview.notConfiguredOwner : t.overview.notConfiguredOther,
                ]
              : status.launch === 'noBuild'
                ? [t.overview.noBuild, t.overview.noBuildHint]
                : [t.overview.invalid, me.isOwner ? t.overview.invalidOwner : t.overview.invalidOther];

    return (
        <>
            <output className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-state">
                <span className={cn('led size-2.5', STATE_LED[status.state])} aria-hidden="true" />
                <span className={cn('font-semibold', STATE_INK[status.state])}>
                    {t.server_state.states[status.state]}.
                </span>
                {cause !== null && <span className="text-ink-3">{cause}</span>}
            </output>
            {facts.length > 0 && <p className="text-small text-ink-3">{facts.join(' · ')}</p>}
            {me.isOwner && <HangAdvice status={status} settings={launchSettings.data?.settings ?? null} />}
            {allows(me, Permission.ChangesView) && <ChangesFact />}
            {launch !== null && (
                <div className="flex flex-col items-start gap-2 text-small text-ink-2">
                    <p>
                        {launch[0]} {launch[1]}
                    </p>
                    {status.launch === 'noBuild' && (
                        <Link
                            to="/artifacts"
                            className="inline-flex items-center gap-1.5 font-medium text-ink underline-offset-2 hover:underline"
                        >
                            {t.overview.openArtifacts}
                            <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                        </Link>
                    )}
                    {me.isOwner && status.launch !== 'noBuild' && (
                        <Link
                            to={status.launch === 'notConfigured' ? '/server/setup' : '/server'}
                            className="inline-flex items-center gap-1.5 font-medium text-ink underline-offset-2 hover:underline"
                        >
                            {status.launch === 'notConfigured' ? t.overview.openSetup : t.overview.openServer}
                            <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                        </Link>
                    )}
                </div>
            )}
        </>
    );
}
