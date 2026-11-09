import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { CircleArrowUp, Download, ExternalLink, RefreshCw, X } from 'lucide-react';
import { useEffect, useState } from 'react';

import { ApiError } from '../api/client';
import type { CheckFailure } from '../api/generated/CheckFailure';
import type { Release } from '../api/generated/Release';
import type { UpdateFailure } from '../api/generated/UpdateFailure';
import type { UpdateJob } from '../api/generated/UpdateJob';
import type { UpdateOutcome } from '../api/generated/UpdateOutcome';
import type { UpdatesView } from '../api/generated/UpdatesView';
import type { UpdateStep } from '../api/generated/UpdateStep';
import type { UpdateWhen } from '../api/generated/UpdateWhen';
import { cancelUpdate, checkUpdates, installUpdate, isUpdating, updatesQuery } from '../api/updates';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Dialog } from '../components/Dialog';
import { Notice } from '../components/Notice';
import { downloadFigures, ProgressBar } from '../components/Progress';
import { Segmented } from '../components/Segmented';
import { Steps } from '../components/Steps';
import type { Messages } from '../i18n/en';
import { errorMessage } from '../i18n/errors';
import { formatClock, formatDay } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { useNow } from '../lib/useNow';
import { useSchedule } from '../schedule/context';
import { useServerStatus } from '../server/context';
import { isRunning } from '../server/state';
import { Markdown } from './Markdown';

/** How long the last install's outcome stays on the card. */
const OUTCOME_DAYS = 7;

/** Followed every second while an install runs, every minute otherwise. */
const followed = {
    ...updatesQuery,
    refetchInterval: (query: { state: { data: UpdatesView | undefined } }) =>
        isUpdating(query.state.data) ? 1000 : 60_000,
};

/**
 * Observer's own updates, for the owner: the newest release with its notes, and its install, now
 * or at the next scheduled restart, with how it goes and how the last one ended.
 */
export function UpdatesCard() {
    const { t, format } = useI18n();
    const client = useQueryClient();
    const updates = useQuery(followed);
    const check = useMutation({
        mutationFn: checkUpdates,
        onSuccess: (view) => client.setQueryData(updatesQuery.queryKey, view),
    });
    const view = updates.data;
    const off = updates.error instanceof ApiError && updates.error.code === 'feature_disabled';
    // The version an install under way here goes to: once Observer stops answering it restarts,
    // and once an Observer answers without that install, the new one runs, with its own panel.
    const [installing, setInstalling] = useState<string | null>(null);
    const running = view?.job?.state === 'running' ? view.job.version : null;
    if (running !== null && running !== installing) {
        setInstalling(running);
    }
    const restarting = installing !== null && (updates.isError || view?.job?.step === 'restart');
    const replaced = installing !== null && !updates.isError && view !== undefined && view.job === null;
    useEffect(() => {
        if (replaced) {
            window.location.reload();
        }
    }, [replaced]);
    const meta =
        view === undefined
            ? undefined
            : view.checking || check.isPending
              ? t.updates.checking
              : view.checkedAt === null
                ? t.updates.neverChecked
                : t.updates.checked(format.timestamp(view.checkedAt));

    return (
        <div id="updates" className="scroll-mt-4">
            <Card
                id="about-updates"
                icon={CircleArrowUp}
                title={t.updates.title}
                meta={meta}
                aside={
                    view !== undefined && (
                        <Button
                            size="sm"
                            tone="ghost"
                            icon={RefreshCw}
                            pending={check.isPending}
                            disabled={isUpdating(view)}
                            onClick={() => check.mutate()}
                        >
                            {t.updates.check}
                        </Button>
                    )
                }
            >
                {updates.isPending && (
                    <div className="flex flex-col gap-3" aria-busy="true">
                        <span className="sr-only">{t.common.loading}</span>
                        <span className="skeleton w-1/2" />
                    </div>
                )}
                {off && (
                    <Notice>
                        {t.updates.off}{' '}
                        <Link
                            to="/settings/$category"
                            params={{ category: 'features' }}
                            className="font-medium underline-offset-2 hover:underline"
                        >
                            {t.updates.turnOn}
                        </Link>
                    </Notice>
                )}
                {updates.isError && !off && view === undefined && !restarting && (
                    <Notice tone="danger">{errorMessage(t, updates.error)}</Notice>
                )}
                {check.isError && <Notice tone="danger">{errorMessage(t, check.error)}</Notice>}
                {restarting || replaced ? (
                    <Notice live={false}>
                        {replaced ? t.updates.reloading : t.updates.restarting(installing ?? '')}
                    </Notice>
                ) : (
                    view !== undefined && <Body view={view} />
                )}
            </Card>
        </div>
    );
}

function Body({ view }: { view: UpdatesView }) {
    const { t, format } = useI18n();
    const job = view.job;
    const now = useNow(60_000);
    const recent = view.last !== null && now - Date.parse(view.last.at) < OUTCOME_DAYS * 24 * 3600 * 1000;
    return (
        <div className="flex flex-col gap-4">
            {recent && view.last !== null && <Outcome last={view.last} />}
            {job !== null && job.state === 'running' ? (
                <JobProgress job={job} />
            ) : (
                <>
                    {job?.state === 'failed' && job.failure !== null && (
                        <Notice tone="danger">{t.updates.failed(failureText(t, job.failure))}</Notice>
                    )}
                    {job?.state === 'cancelled' && <Notice>{t.updates.cancelled}</Notice>}
                    {view.available !== null ? (
                        <ReleaseView release={view.available} view={view} />
                    ) : view.checkFailure !== null ? (
                        <Notice
                            tone={view.checkFailure.code === 'noRelease' ? 'neutral' : 'warning'}
                            live={false}
                        >
                            {checkFailureText(t, format.timestamp, view.checkFailure)}
                        </Notice>
                    ) : (
                        view.checkedAt !== null && (
                            <p className="text-body text-ink-2">{t.updates.upToDate(view.current)}</p>
                        )
                    )}
                </>
            )}
        </div>
    );
}

function Outcome({ last }: { last: UpdateOutcome }) {
    const { t, format } = useI18n();
    const at = format.timestamp(last.at);
    return last.result.kind === 'installed' ? (
        <Notice tone="success" live={false}>
            {t.updates.last.installed(last.from, last.to, at)}
        </Notice>
    ) : (
        <Notice tone="danger" live={false}>
            {t.updates.last.rolledBack(last.from, last.to, at)}{' '}
            {t.updates.rollbackReasons[last.result.reason]}
        </Notice>
    );
}

function ReleaseView({ release, view }: { release: Release; view: UpdatesView }) {
    const { t, format } = useI18n();
    const facts = [
        release.publishedAt === null
            ? null
            : t.updates.published(
                  format.dateTime(new Date(release.publishedAt), {
                      dateStyle: 'medium',
                      timeStyle: undefined,
                  }),
              ),
        release.asset === null ? null : format.bytes(release.asset.size),
        release.signed ? t.updates.signed : t.updates.unsigned,
    ].filter((fact) => fact !== null);
    return (
        <div className="flex flex-col gap-4">
            <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1">
                <span className="font-mono text-body font-medium text-ink">
                    {t.updates.from(view.current, release.version)}
                </span>
                <span className="text-meta text-ink-3">{facts.join(' · ')}</span>
                <a
                    href={release.url}
                    target="_blank"
                    rel="noreferrer noopener"
                    className="inline-flex items-center gap-1 text-small font-medium text-ink-2 underline-offset-2 hover:text-ink hover:underline md:ml-auto max-md:min-h-11"
                >
                    {t.updates.onGithub}
                    <ExternalLink size={12} strokeWidth={1.75} aria-hidden="true" />
                </a>
            </div>
            <section aria-label={t.updates.notes} className="rounded-box border border-line px-4 py-3">
                {release.notes.trim() === '' ? (
                    <p className="text-body text-ink-3">{t.updates.noNotes}</p>
                ) : (
                    <Markdown source={release.notes} />
                )}
                {release.notesCut && <p className="mt-3 text-small text-ink-3">{t.updates.notesCut}</p>}
            </section>
            {view.blocked !== null ? (
                <Notice tone="warning" live={false}>
                    {t.updates.blocked[view.blocked]}
                </Notice>
            ) : release.asset === null ? (
                <Notice tone="warning" live={false}>
                    {t.updates.noAsset}
                </Notice>
            ) : !release.signed ? (
                <Notice tone="warning" live={false}>
                    {t.updates.unsignedRelease}
                </Notice>
            ) : (
                <Install release={release} view={view} />
            )}
        </div>
    );
}

/** A restart's time in the schedule's zone, which the panel says. */
function useRestartTime() {
    const { t, locale } = useI18n();
    const { schedule } = useSchedule();
    return (at: string) => {
        const zone = schedule?.zone ?? 'UTC';
        const day = formatDay(locale, at, zone, { weekday: true });
        return `${t.schedule.dayTime(day, formatClock(locale, at, zone))} (${zone})`;
    };
}

function Install({ release, view }: { release: Release; view: UpdatesView }) {
    const { t } = useI18n();
    const client = useQueryClient();
    const { status } = useServerStatus();
    const restartTime = useRestartTime();
    const running = status !== null && isRunning(status.state);
    const restartAt = running ? view.nextRestartAt : null;
    const choices: UpdateWhen[] = restartAt === null ? ['now'] : ['now', 'scheduledRestart'];
    const [chosen, setChosen] = useState<UpdateWhen>('now');
    const when = choices.includes(chosen) ? chosen : 'now';
    const [confirming, setConfirming] = useState(false);
    const install = useMutation({
        mutationFn: () => installUpdate({ version: release.version, when }),
        onSuccess: (next) => {
            client.setQueryData(updatesQuery.queryKey, next);
            setConfirming(false);
        },
    });
    const about = !running
        ? t.updates.whenAbout.stopped
        : when === 'scheduledRestart' && restartAt !== null
          ? t.updates.whenAbout.scheduledRestart(restartTime(restartAt))
          : t.updates.whenAbout.now;
    const confirm = !running
        ? t.updates.confirmStopped
        : when === 'scheduledRestart' && restartAt !== null
          ? t.updates.confirmAtRestart(restartTime(restartAt))
          : t.updates.confirmNow;

    return (
        <div className="flex flex-col gap-3">
            {running && (
                <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                    <span className="text-meta text-ink-3">{t.updates.when}</span>
                    <Segmented
                        label={t.updates.when}
                        choices={choices}
                        value={when}
                        words={t.updates.whenWords}
                        onChange={setChosen}
                    />
                </div>
            )}
            <p className="text-small text-ink-2">{about}</p>
            {running && restartAt === null && <p className="text-small text-ink-3">{t.updates.noRestart}</p>}
            <div>
                <Button tone="ink" icon={Download} onClick={() => setConfirming(true)}>
                    {t.updates.install(release.version)}
                </Button>
            </div>
            {confirming && (
                <Dialog
                    title={t.updates.confirm(release.version)}
                    onClose={() => setConfirming(false)}
                    onSubmit={() => install.mutate()}
                    footer={
                        <>
                            <Button tone="ghost" onClick={() => setConfirming(false)}>
                                {t.common.cancel}
                            </Button>
                            <Button type="submit" tone="ink" icon={Download} pending={install.isPending}>
                                {t.updates.install(release.version)}
                            </Button>
                        </>
                    }
                >
                    <div className="flex flex-col gap-3 text-body text-ink-2">
                        <p>{confirm}</p>
                        <p className="text-small text-ink-3">{t.updates.rollbackNote}</p>
                        {install.isError && <Notice tone="danger">{errorMessage(t, install.error)}</Notice>}
                    </div>
                </Dialog>
            )}
        </div>
    );
}

function JobProgress({ job }: { job: UpdateJob }) {
    const { t, format } = useI18n();
    const client = useQueryClient();
    const restartTime = useRestartTime();
    const steps: UpdateStep[] =
        job.when === 'scheduledRestart'
            ? ['download', 'verify', 'wait', 'restart']
            : ['download', 'verify', 'restart'];
    const at = steps.indexOf(job.step);
    const cancel = useMutation({
        mutationFn: cancelUpdate,
        onSuccess: (next) => client.setQueryData(updatesQuery.queryKey, next),
    });
    const download = job.step === 'download' ? job.download : null;
    const figures = download === null ? [] : downloadFigures(t, format, download);
    return (
        <div className="flex flex-col gap-4">
            <Steps
                label={t.updates.stepsLabel(job.version)}
                steps={steps.map((step, index) => ({
                    key: step,
                    label: t.updates.steps[step],
                    passed: index < at,
                    failed: false,
                }))}
                current={at}
                busy={job.step !== 'wait'}
            />
            {job.step === 'download' && (
                <>
                    <ProgressBar
                        label={t.updates.progress}
                        valueText={figures.join(' · ') || t.updates.steps.download}
                        share={
                            download === null || download.total === 0
                                ? null
                                : download.received / download.total
                        }
                    />
                    {figures.length > 0 && (
                        <p className="text-small text-ink-2 tabular-nums">{figures.join(' · ')}</p>
                    )}
                </>
            )}
            {job.step === 'verify' && <p className="text-small text-ink-2">{t.updates.verifying}</p>}
            {job.step === 'wait' && (
                <p className="text-small text-ink-2">
                    {job.restartAt === null
                        ? t.updates.waitingUnknown
                        : t.updates.waiting(restartTime(job.restartAt))}
                </p>
            )}
            {job.step !== 'restart' && (
                <div>
                    <Button size="sm" icon={X} pending={cancel.isPending} onClick={() => cancel.mutate()}>
                        {t.updates.cancel}
                    </Button>
                    {cancel.isError && <Notice tone="danger">{errorMessage(t, cancel.error)}</Notice>}
                </div>
            )}
        </div>
    );
}

const failureText = (t: Messages, failure: UpdateFailure) => {
    const words = t.updates.failures;
    switch (failure.code) {
        case 'httpStatus':
            return words.httpStatus(failure.status);
        case 'diskSpace':
            return words.diskSpace(`${failure.neededMib} MiB`, `${failure.availableMib} MiB`);
        default:
            return words[failure.code];
    }
};

const checkFailureText = (t: Messages, timestamp: (rfc3339: string) => string, failure: CheckFailure) => {
    const words = t.updates.checkFailures;
    switch (failure.code) {
        case 'httpStatus':
            return words.httpStatus(failure.status);
        case 'rateLimited':
            return words.rateLimited(failure.until === null ? null : timestamp(failure.until));
        default:
            return words[failure.code];
    }
};
