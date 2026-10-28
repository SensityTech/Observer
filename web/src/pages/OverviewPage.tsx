import { Link } from '@tanstack/react-router';
import { ArrowRight } from 'lucide-react';
import { lazy, Suspense } from 'react';

import type { ServerStatus } from '../api/generated/ServerStatus';
import { useMe } from '../auth/me';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import { cn } from '../lib/cn';
import { secondsSince, useNow } from '../lib/useNow';
import { LiveMetricsProvider, useLiveMetrics } from '../metrics/live';
import { Tiles } from '../overview/Tiles';
import { useServerStatus } from '../server/context';
import { STATE_INK, STATE_LED, causeText, isRunning } from '../server/state';

// The chart library is the heaviest part of the panel: it loads after the page has painted.
const PlayersChart = lazy(() => import('../charts/PlayersChart'));

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
                <Tiles />
                <Suspense fallback={<ChartPlaceholder />}>
                    <PlayersChart />
                </Suspense>
            </LiveMetricsProvider>
        </>
    );
}

/** The chart card's shape while its code loads. */
function ChartPlaceholder() {
    const { t } = useI18n();
    return (
        <section className="card" aria-busy="true" aria-label={t.charts.players.loading}>
            <div className="card-head">
                <span className="skeleton h-3 w-40" />
            </div>
            <div className="card-body">
                <div className="skeleton h-[260px] w-full max-md:h-[200px]" />
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

function ServerState({ status }: { status: ServerStatus }) {
    const { t, format } = useI18n();
    const me = useMe();
    const now = useNow();
    const { latest } = useLiveMetrics();
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
        status.build === null ? null : t.overview.build(String(status.build)),
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
    ].filter((fact) => fact !== null);

    const launch =
        status.launch === 'ready'
            ? null
            : status.launch === 'notConfigured'
              ? [
                    t.overview.notConfigured,
                    me.isOwner ? t.overview.notConfiguredOwner : t.overview.notConfiguredOther,
                ]
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
            {launch !== null && (
                <div className="flex flex-col items-start gap-2 text-small text-ink-2">
                    <p>
                        {launch[0]} {launch[1]}
                    </p>
                    {me.isOwner && (
                        <Link
                            to="/server"
                            className="inline-flex items-center gap-1.5 font-medium text-ink underline-offset-2 hover:underline"
                        >
                            {t.overview.openServer}
                            <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                        </Link>
                    )}
                </div>
            )}
        </>
    );
}
