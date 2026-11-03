import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, ChevronRight, RotateCw } from 'lucide-react';
import { Suspense, useId, useState } from 'react';

import type { ServerState } from '../api/generated/ServerState';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { launchQuery } from '../api/launch';
import { growthQuery } from '../api/metrics';
import { useMe } from '../auth/me';
import { Button } from '../components/Button';
import { ErrorBoundary } from '../components/ErrorBoundary';
import { useAlerts } from '../diagnostics/alerts';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import { cn } from '../lib/cn';
import { lazyCode } from '../lib/lazyCode';
import { useNow } from '../lib/useNow';
import { LiveMetricsProvider } from '../metrics/live';
import { ComingUp, Recent } from '../overview/Agenda';
import { Alerts } from '../overview/Alerts';
import { Summary } from '../overview/Summary';
import { ThreadsCard } from '../overview/ThreadsCard';
import { Tiles } from '../overview/Tiles';
import { TreeCard } from '../overview/TreeCard';
import { useServerStatus } from '../server/context';
import { HangAdvice } from '../server/HangAdvice';
import { STATE_INK, STATE_LED, causeText } from '../server/state';

// The chart library is the heaviest part of the panel: it loads after the page has painted.
const PlayersChart = lazyCode(() => import('../charts/PlayersChart'));
const TickChart = lazyCode(() => import('../charts/TickChart'));

/** States that are a problem in themselves, whatever caused them. */
const TROUBLED: ReadonlySet<ServerState> = new Set(['degraded', 'hung', 'crashed', 'crashLoop']);

/** The server's state needs a notice above the figures: a troubled state, or launch settings to fix. */
const needsNotice = (status: ServerStatus | null) =>
    status !== null && (TROUBLED.has(status.state) || status.launch !== 'ready');

export function OverviewPage() {
    const { t } = useI18n();
    const { status } = useServerStatus();
    const notice = needsNotice(status);

    return (
        <LiveMetricsProvider>
            <div className="page-head">
                <h1 className="page-title">
                    {t.overview.title} <Verdict />
                </h1>
            </div>
            <Alerts />
            {status !== null && notice && <StateNotice status={status} />}
            <Summary />
            <ErrorBoundary
                fallback={<ChartFailed title={t.charts.players.title} text={t.charts.players.unavailable} />}
            >
                <Suspense fallback={<ChartPlaceholder label={t.charts.players.loading} height="h-[260px]" />}>
                    <PlayersChart />
                </Suspense>
            </ErrorBoundary>
            <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
                <ComingUp />
                <Recent causeShown={notice} />
            </div>
            <Details />
        </LiveMetricsProvider>
    );
}

/**
 * The page title's gloss: everything is fine, how many things need a look (live alerts, a troubled
 * state, launch settings to fix, a possible leak), or the state the server is passing through.
 */
function Verdict() {
    const { t } = useI18n();
    const words = t.overview.verdict;
    const { status } = useServerStatus();
    const alerts = useAlerts();
    const growth = useQuery(growthQuery);
    const leaking = growth.data?.private?.leak === true || growth.data?.handles?.leak === true;

    let gloss: string = words.unknown;
    if (status !== null) {
        const things =
            alerts.length +
            Number(TROUBLED.has(status.state)) +
            Number(status.launch !== 'ready') +
            Number(leaking);
        if (things > 0) {
            gloss = words.look(things);
        } else if (status.state === 'stopped' || status.state === 'starting' || status.state === 'stopping') {
            gloss = words[status.state];
        } else {
            gloss = words.fine;
        }
    }
    return <span className="gloss">· {gloss}</span>;
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

/**
 * A troubled state or launch settings to fix, above the figures: the state in a word and why, the
 * restart under way, and what to do about it.
 */
function StateNotice({ status }: { status: ServerStatus }) {
    const { t, format } = useI18n();
    const me = useMe();
    const now = useNow();
    const link = useLinkState();
    // Only the owner reads the launch settings, and can change the delay a hang names.
    const launchSettings = useQuery({ ...launchQuery, enabled: me.isOwner });
    const cause = causeText(t, status);

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
        <section className="card" aria-labelledby="overview-state">
            <div className="card-body flex flex-col gap-3 max-md:p-4">
                <h2 id="overview-state" className="eyebrow">
                    {t.overview.stateTitle}
                </h2>
                <output
                    className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-state"
                    aria-busy={link === 'connecting' || undefined}
                >
                    <span className={cn('led size-2.5', STATE_LED[status.state])} aria-hidden="true" />
                    <span className={cn('font-semibold', STATE_INK[status.state])}>
                        {t.server_state.states[status.state]}.
                    </span>
                    {cause !== null && <span className="text-ink-3">{cause}</span>}
                </output>
                {status.restartAt !== null && (
                    <p className="text-small text-ink-3">
                        {t.server_state.restartIn(
                            format.duration(Math.max(0, Date.parse(status.restartAt) - now) / 1000),
                        )}
                    </p>
                )}
                {me.isOwner && (
                    <HangAdvice status={status} settings={launchSettings.data?.settings ?? null} />
                )}
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
                                {status.launch === 'notConfigured'
                                    ? t.overview.openSetup
                                    : t.overview.openServer}
                                <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                            </Link>
                        )}
                    </div>
                )}
            </div>
        </section>
    );
}

/**
 * Every measure of FXServer, the host and Observer, folded: the Overview keeps four figures, and
 * these wait here until a page of their own holds them. Their reads start when it opens.
 */
function Details() {
    const { t } = useI18n();
    const [open, setOpen] = useState(false);
    const panelId = useId();
    return (
        <div className="flex flex-col gap-3">
            <button
                type="button"
                className="inline-flex items-center gap-2 self-start rounded-sm py-1 pr-2 text-ui font-medium text-ink-2 hover:text-ink max-md:min-h-11"
                aria-expanded={open}
                aria-controls={panelId}
                onClick={() => setOpen(!open)}
            >
                <ChevronRight
                    size={16}
                    strokeWidth={1.5}
                    className={cn('transition-transform', open && 'rotate-90')}
                    aria-hidden="true"
                />
                {t.overview.details}
                <span className="font-normal text-ink-3">· {t.overview.detailsHint}</span>
            </button>
            <section
                id={panelId}
                aria-label={t.overview.details}
                hidden={!open}
                className="flex flex-col gap-3"
            >
                {open && (
                    <>
                        <Tiles />
                        <div className="grid grid-cols-1 gap-3 2xl:grid-cols-3">
                            <div className="flex min-w-0 flex-col 2xl:col-span-2">
                                <ErrorBoundary
                                    fallback={
                                        <ChartFailed
                                            title={t.charts.tick.title}
                                            text={t.charts.tick.unavailable}
                                        />
                                    }
                                >
                                    <Suspense
                                        fallback={
                                            <ChartPlaceholder
                                                label={t.charts.tick.loading}
                                                height="h-[240px]"
                                            />
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
                    </>
                )}
            </section>
        </div>
    );
}
