import { useMutation, useQuery, useQueryClient, type UseQueryResult } from '@tanstack/react-query';
import {
    Circle,
    CircleCheck,
    CircleX,
    Clock,
    Copy,
    Download,
    FolderInput,
    HardDrive,
    LoaderCircle,
    Package,
    Plus,
    RefreshCw,
    Search,
    Settings2,
    Trash2,
    X,
    type LucideIcon,
} from 'lucide-react';
import { useId, useState, type ReactNode } from 'react';

import {
    KVP_BUILD,
    cancelJob,
    cancelPending,
    catalogQuery,
    cleanUp,
    cleanupQuery,
    defaultMode,
    deleteBuild,
    importBuild,
    installBuild,
    needsKvpConfirmation,
    protectionsOf,
    refreshCatalog,
    saveArtifactSettings,
    secondsLeft,
    selectBuild,
    settingsIssues,
    stepsOf,
} from '../api/artifacts';
import { ApiError } from '../api/client';
import type { ApplyMode } from '../api/generated/ApplyMode';
import type { ArtifactNotice } from '../api/generated/ArtifactNotice';
import type { ArtifactSettings } from '../api/generated/ArtifactSettings';
import type { ArtifactSettingsIssue } from '../api/generated/ArtifactSettingsIssue';
import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import type { CatalogEntry } from '../api/generated/CatalogEntry';
import type { CatalogView } from '../api/generated/CatalogView';
import type { CatalogWarning } from '../api/generated/CatalogWarning';
import type { InstalledBuild } from '../api/generated/InstalledBuild';
import type { InstallFailure } from '../api/generated/InstallFailure';
import type { Job } from '../api/generated/Job';
import { Permission } from '../api/generated/Permission';
import type { PolicyDay } from '../api/generated/PolicyDay';
import { launchProblem, launchQuery } from '../api/launch';
import { useArtifacts } from '../artifacts/context';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Checkbox } from '../components/Checkbox';
import { Chip } from '../components/Chip';
import { Dialog } from '../components/Dialog';
import { FieldError, TextField, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { errorMessage } from '../i18n/errors';
import { formatClock, formatDay, knownZone, zoneOffset } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import { cn } from '../lib/cn';
import { useNow } from '../lib/useNow';
import { useSchedule } from '../schedule/context';
import { useServerStatus } from '../server/context';
import { isRunning } from '../server/state';

type Outcome = { tone: 'success' | 'warning' | 'danger'; message: string };

type Open =
    | { kind: 'install'; entry: CatalogEntry }
    | { kind: 'use'; build: number }
    | { kind: 'delete'; build: number }
    | { kind: 'cleanup' }
    | { kind: 'import' };

const HIDDEN_KEY = 'observer.artifacts.hiddenNotices';
const LOW_DISK = 1024 ** 3;

/** The UTC day of an API timestamp: catalog dates are days. */
const day = (instant: string) => instant.slice(0, 10);

const policyDay = (date: PolicyDay | null, never: string) =>
    date === null ? null : date.kind === 'never' ? never : day(date.at);

const readHidden = (): string[] => {
    try {
        const stored: unknown = JSON.parse(localStorage.getItem(HIDDEN_KEY) ?? '[]');
        return Array.isArray(stored) ? stored.filter((each) => typeof each === 'string') : [];
    } catch {
        return [];
    }
};

/**
 * The clock and day of server times, in the schedule's zone as every server time is: the last one
 * known while the link is down, UTC only before any, which the page head then says.
 */
const useZone = () => {
    const { locale, t } = useI18n();
    const zone = useSchedule().zone ?? 'UTC';
    return {
        clock: (instant: string) => formatClock(locale, instant, zone),
        when: (instant: string) =>
            t.schedule.dayTime(
                formatDay(locale, instant, zone, { weekday: false }),
                formatClock(locale, instant, zone),
            ),
    };
};

export function ArtifactsPage() {
    const { t } = useI18n();
    const link = useLinkState();
    const { status: live, refused } = useArtifacts();
    // While the link is down the page keeps the last status it had, so that a form keeps what is typed.
    const [last, setLast] = useState(live);
    if (live !== null && live !== last) {
        setLast(live);
    }
    const status = live ?? (refused ? null : last);
    const [open, setOpen] = useState<Open | null>(null);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const catalog = useQuery(catalogQuery);
    const close = () => setOpen(null);
    const done = (message: string) => {
        setOpen(null);
        setOutcome({ tone: 'success', message });
    };

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.artifacts.title} <span className="gloss">{t.artifacts.gloss}</span>
                </h1>
                <ZoneNote />
            </div>
            {status !== null && live === null && <Notice tone="warning">{t.artifacts.outOfDate}</Notice>}
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            {status === null ? (
                <section className="card">
                    <div className="card-body flex flex-col gap-3" aria-busy={link !== 'offline'}>
                        {link === 'live' || link === 'connecting' ? (
                            <>
                                <span className="sr-only">{t.artifacts.loading}</span>
                                <span className="skeleton w-1/3" />
                                <span className="skeleton w-1/2" />
                            </>
                        ) : (
                            <p className="text-ink-3">{t.artifacts.unknown}</p>
                        )}
                    </div>
                </section>
            ) : (
                <>
                    <Notices
                        status={status}
                        warnings={catalog.data?.warnings ?? []}
                        onImport={() => setOpen({ kind: 'import' })}
                    />
                    <BuildCard status={status} onOutcome={setOutcome} />
                    {status.job !== null && (
                        <JobCard
                            job={status.job}
                            onOutcome={setOutcome}
                            onUse={(build) => setOpen({ kind: 'use', build })}
                            onRetry={(build) => {
                                const entry = catalog.data?.builds.find((each) => each.build === build);
                                if (entry !== undefined) {
                                    setOpen({ kind: 'install', entry });
                                }
                            }}
                        />
                    )}
                    <InstalledCard
                        status={status}
                        catalog={catalog.data ?? null}
                        onOpen={setOpen}
                        onOutcome={setOutcome}
                    />
                    <AvailableCard
                        status={status}
                        catalog={catalog}
                        onInstall={(entry) => setOpen({ kind: 'install', entry })}
                        onOutcome={setOutcome}
                    />
                    <SettingsCard status={status} />
                    {open?.kind === 'install' && (
                        <InstallDialog entry={open.entry} status={status} onClose={close} onDone={done} />
                    )}
                    {open?.kind === 'use' && (
                        <UseDialog
                            build={open.build}
                            status={status}
                            catalog={catalog.data ?? null}
                            onClose={close}
                            onDone={done}
                        />
                    )}
                    {open?.kind === 'delete' && (
                        <DeleteDialog
                            build={open.build}
                            status={status}
                            catalog={catalog.data ?? null}
                            onClose={close}
                            onDone={done}
                        />
                    )}
                    {open?.kind === 'cleanup' && <CleanupDialog onClose={close} onDone={done} />}
                    {open?.kind === 'import' && <ImportDialog onClose={close} onDone={done} />}
                </>
            )}
        </div>
    );
}

/** Which zone the times of the page read in, and that its days are UTC. */
function ZoneNote() {
    const { t, locale } = useI18n();
    const { zone } = useSchedule();
    const now = useNow(60_000);
    const times =
        zone === null
            ? t.artifacts.utcTimes
            : knownZone(zone) === zone
              ? t.schedule.zone(zone, zoneOffset(locale, zone, now))
              : t.schedule.zoneUnknown(zone);
    return (
        <span className="inline-flex items-center gap-1.5 text-ui text-ink-3">
            <Clock size={14} strokeWidth={1.5} aria-hidden="true" />
            {t.artifacts.zone(times)}
        </span>
    );
}

function Card({
    id,
    icon: Icon,
    title,
    aside,
    flush = false,
    children,
}: {
    id: string;
    icon: LucideIcon;
    title: string;
    aside?: ReactNode;
    /** The body runs to the card's edges, as a table does. */
    flush?: boolean;
    children: ReactNode;
}) {
    return (
        <section className="card" aria-labelledby={id}>
            <div className="card-head flex-wrap">
                <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                    <Icon size={14} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <h2 id={id} className="text-card text-ink">
                    {title}
                </h2>
                {aside !== undefined && (
                    <div className="ml-auto flex flex-wrap items-center gap-2">{aside}</div>
                )}
            </div>
            <div className={flush ? '' : 'p-5 max-md:p-4'}>{children}</div>
        </section>
    );
}

/** A control the account cannot use, or that does nothing now, stays visible and says why. */
function Action({
    label,
    icon,
    reason,
    tone,
    size,
    pending,
    onPress,
    onBlocked,
}: {
    label: string;
    icon?: LucideIcon;
    reason: string | null;
    tone?: 'ink' | 'secondary' | 'ghost' | 'danger';
    size?: 'md' | 'sm';
    pending?: boolean;
    onPress: () => void;
    onBlocked: (reason: string) => void;
}) {
    return (
        <Button
            icon={icon}
            tone={tone}
            size={size}
            pending={pending}
            aria-disabled={reason !== null || undefined}
            title={reason ?? undefined}
            className={cn(
                'max-md:w-full',
                reason !== null && 'cursor-not-allowed opacity-45 shadow-none active:translate-y-0',
            )}
            onClick={() => (reason === null ? onPress() : onBlocked(reason))}
        >
            {label}
        </Button>
    );
}

function Notices({
    status,
    warnings,
    onImport,
}: {
    status: ArtifactsStatus;
    warnings: CatalogWarning[];
    onImport: () => void;
}) {
    const { t, format } = useI18n();
    const me = useMe();
    const zone = useZone();
    const [hidden, setHidden] = useState(readHidden);
    const words = t.artifacts.notices;
    const notice = status.notice;
    const hide = (at: string) => {
        const next = [...hidden.filter((each) => each !== at), at].slice(-20);
        setHidden(next);
        try {
            localStorage.setItem(HIDDEN_KEY, JSON.stringify(next));
        } catch {
            // Without storage the notice stays hidden for this visit only.
        }
    };
    const source = (warning: CatalogWarning) => {
        if (warning.kind === 'commitMismatch') {
            return '';
        }
        return warning.source.kind === 'branch'
            ? words.branch(warning.source.name)
            : warning.source.kind === 'docsPage'
              ? words.docsPage
              : words.changelog;
    };
    return (
        <>
            {notice !== null && !hidden.includes(notice.at) && (
                <Notice tone={notice.kind === 'rolledBack' ? 'warning' : 'danger'}>
                    <div className="flex flex-wrap items-start gap-x-3 gap-y-1">
                        <span className="min-w-0 flex-1">{noticeText(t, notice)}</span>
                        <span className="text-meta">{zone.when(notice.at)}</span>
                        <button
                            type="button"
                            className="inline-flex items-center gap-1 text-meta font-medium underline-offset-2 hover:underline max-md:min-h-11 max-md:px-1"
                            aria-label={words.hideLabel}
                            onClick={() => hide(notice.at)}
                        >
                            <X size={12} strokeWidth={2} aria-hidden="true" />
                            {words.hide}
                        </button>
                    </div>
                </Notice>
            )}
            {status.legacyFolder && me.isOwner && (
                <Notice tone="warning">
                    <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                        <span className="min-w-0 flex-1">{words.legacy}</span>
                        <Button size="sm" icon={FolderInput} onClick={onImport}>
                            {words.importLegacy}
                        </Button>
                    </div>
                </Notice>
            )}
            {warnings.map((warning) => (
                <Notice key={JSON.stringify(warning)} tone="warning">
                    {warning.kind === 'stale'
                        ? words.stale(source(warning), zone.when(warning.since))
                        : warning.kind === 'missing'
                          ? words.missing(source(warning))
                          : words.commitMismatch(String(warning.build))}
                </Notice>
            ))}
            {status.longPath !== null && (
                <Notice tone="warning">{words.longPath(format.number(status.longPath))}</Notice>
            )}
            {status.disk !== null && status.disk.free < LOW_DISK && (
                <Notice tone="warning">{words.lowDisk(format.bytes(status.disk.free))}</Notice>
            )}
        </>
    );
}

const noticeText = (t: ReturnType<typeof useI18n>['t'], notice: ArtifactNotice) => {
    const words = t.artifacts.notices;
    const build = String(notice.build);
    switch (notice.kind) {
        case 'rolledBack':
            return words.rolledBack(String(notice.other ?? ''), build, notice.reason);
        case 'probationFailed':
            return words.probationFailed(build, notice.reason, String(notice.other ?? ''));
        case 'pendingDropped':
            return words.pendingDropped(build, notice.reason);
    }
};

function Mono({ build }: { build: number }) {
    return <span className="font-mono">{build}</span>;
}

/** Copies a commit where the browser allows it; its full text stays in the title otherwise. */
const copyText = async (text: string) => {
    try {
        await navigator.clipboard.writeText(text);
    } catch {
        // No clipboard outside a secure context: the title still shows it.
    }
};

function Fact({ label, value, after }: { label: string; value: ReactNode; after?: ReactNode }) {
    return (
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
            <dt className="text-ui text-ink-3">{label}</dt>
            <dd className="flex flex-wrap items-baseline gap-x-2 text-body text-ink">{value}</dd>
            {after}
        </div>
    );
}

function BuildCard({
    status,
    onOutcome,
}: {
    status: ArtifactsStatus;
    onOutcome: (outcome: Outcome) => void;
}) {
    const { t } = useI18n();
    const me = useMe();
    const zone = useZone();
    const { schedule } = useSchedule();
    const id = useId();
    const words = t.artifacts.build;
    const canManage = allows(me, Permission.ArtifactsManage);
    const cancel = useMutation({
        mutationFn: cancelPending,
        onSuccess: () => onOutcome({ tone: 'success', message: words.pendingCancelled }),
        onError: (error) =>
            onOutcome({
                tone: 'danger',
                message: t.artifacts.failed(words.cancelPending, errorMessage(t, error)),
            }),
    });
    const next = schedule?.next ?? null;
    const probation = status.probation;
    return (
        <Card id={id} icon={Package} title={words.title}>
            <dl className="flex flex-col gap-2.5">
                <Fact
                    label={status.running === null ? words.stopped : words.running}
                    value={status.running === null ? null : <Mono build={status.running} />}
                />
                <Fact
                    label={words.nextStart}
                    value={
                        status.selected === null ? (
                            <span className="text-ink-3">{words.none}</span>
                        ) : (
                            <Mono build={status.selected} />
                        )
                    }
                />
                {status.pending !== null && (
                    <Fact
                        label={words.pending}
                        value={
                            <>
                                <Mono build={status.pending.build} />
                                <span className="text-ink-3">
                                    ·{' '}
                                    {next === null
                                        ? words.atRestartUnknown
                                        : words.atRestart(zone.clock(next.at))}
                                </span>
                            </>
                        }
                        after={
                            <Action
                                label={words.cancelPending}
                                size="sm"
                                tone="ghost"
                                pending={cancel.isPending}
                                reason={canManage ? null : t.artifacts.installed.notPermitted}
                                onPress={() => cancel.mutate()}
                                onBlocked={(reason) => onOutcome({ tone: 'warning', message: reason })}
                            />
                        }
                    />
                )}
                {status.lastHealthy !== null && (
                    <Fact
                        label={words.lastHealthy}
                        value={
                            <>
                                <Mono build={status.lastHealthy.build} />
                                <span className="text-ink-3">
                                    · {words.since(zone.when(status.lastHealthy.since))}
                                </span>
                            </>
                        }
                    />
                )}
                {probation !== null && (
                    <Fact
                        label={words.probation}
                        value={
                            <>
                                <Mono build={probation.build} />
                                <span className="text-ink-3">
                                    ·{' '}
                                    {probation.passesAt === null
                                        ? words.waiting
                                        : words.passesAt(zone.clock(probation.passesAt))}
                                </span>
                            </>
                        }
                    />
                )}
            </dl>
            {status.running !== null && status.selected !== null && status.running !== status.selected && (
                <p className="mt-3 text-small text-warning">
                    {words.restartNeeded(String(status.running), String(status.selected))}
                </p>
            )}
        </Card>
    );
}

const failureText = (
    t: ReturnType<typeof useI18n>['t'],
    failure: InstallFailure,
    bytes: (n: number) => string,
) => {
    const words = t.artifacts.job;
    switch (failure.code) {
        case 'httpStatus':
            return words.httpStatus(failure.status);
        case 'diskSpace':
            return words.diskSpace(
                bytes(failure.neededMib * 1024 * 1024),
                bytes(failure.availableMib * 1024 * 1024),
            );
        default:
            return words.failures[failure.code];
    }
};

export function JobCard({
    job,
    onOutcome,
    onUse,
    onRetry,
}: {
    job: Job;
    onOutcome: (outcome: Outcome) => void;
    onUse: (build: number) => void;
    onRetry: (build: number) => void;
}) {
    const { t, format } = useI18n();
    const me = useMe();
    const id = useId();
    const words = t.artifacts.job;
    const steps = stepsOf(job.kind);
    const current = steps.indexOf(job.step);
    const running = job.state === 'running';
    const cancel = useMutation({
        mutationFn: cancelJob,
        onError: (error) =>
            onOutcome({ tone: 'danger', message: t.artifacts.failed(words.cancel, errorMessage(t, error)) }),
    });
    const download = job.download;
    const entries = job.entries;
    const share =
        job.step === 'download' && download !== null && download.total > 0
            ? download.received / download.total
            : entries !== null && entries.total !== null && entries.total > 0
              ? entries.done / entries.total
              : null;
    const left = download === null ? null : secondsLeft(download);
    // The download's figures while it lasts; the files after it.
    const downloading = job.step === 'download' && download !== null ? download : null;
    const progress = [
        downloading === null
            ? null
            : words.downloaded(format.bytes(downloading.received), format.bytes(downloading.total)),
        downloading === null || downloading.rate === 0 ? null : words.rate(format.bytes(downloading.rate)),
        downloading === null || left === null ? null : words.left(format.duration(left)),
        entries === null || job.step === 'download' ? null : words.files(entries.done, entries.total),
    ].filter((part) => part !== null);
    const canManage = allows(me, Permission.ArtifactsManage);
    const cancelReason = !canManage
        ? t.artifacts.installed.notPermitted
        : job.step === 'commit'
          ? words.committing
          : null;
    return (
        <Card
            id={id}
            icon={Download}
            title={
                job.kind === 'install' ? words.install(String(job.build)) : words.import(String(job.build))
            }
        >
            <div className="flex flex-col gap-4">
                <ol className="flex flex-wrap gap-x-4 gap-y-2" aria-label={words.title}>
                    {steps.map((step, index) => {
                        const isCurrent = index === current;
                        const passed = index < current || (job.state === 'installed' && index <= current);
                        const failed = isCurrent && (job.state === 'failed' || job.state === 'cancelled');
                        const Icon = failed
                            ? CircleX
                            : passed
                              ? CircleCheck
                              : isCurrent && running
                                ? LoaderCircle
                                : Circle;
                        return (
                            <li
                                key={step}
                                aria-current={isCurrent ? 'step' : undefined}
                                className={cn(
                                    'inline-flex items-center gap-1.5 text-ui',
                                    isCurrent ? 'font-medium text-ink' : passed ? 'text-ink-2' : 'text-ink-3',
                                    failed && 'text-danger-ink',
                                )}
                            >
                                <Icon
                                    size={14}
                                    strokeWidth={1.75}
                                    className={cn(isCurrent && running && 'animate-spin')}
                                    aria-hidden="true"
                                />
                                {words.steps[step]}
                            </li>
                        );
                    })}
                </ol>
                {running && (
                    <>
                        <progress
                            aria-label={words.progress}
                            aria-valuetext={progress.join(' · ') || words.steps[job.step]}
                            max={100}
                            value={share === null ? undefined : Math.round(share * 100)}
                            className="h-1.5 w-full appearance-none overflow-hidden rounded-pill bg-surface-2 [&::-moz-progress-bar]:bg-accent [&::-webkit-progress-bar]:bg-surface-2 [&::-webkit-progress-value]:rounded-pill [&::-webkit-progress-value]:bg-accent"
                        />
                        <p className="flex flex-wrap gap-x-3 text-small text-ink-2 tabular-nums">
                            {progress.length > 0 && <span>{progress.join(' · ')}</span>}
                        </p>
                        {job.warnings.includes('longPath') && (
                            <p className="text-small text-warning">{words.longPath}</p>
                        )}
                        {job.warnings.includes('freeSpaceUnknown') && (
                            <p className="text-small text-warning">{words.freeSpaceUnknown}</p>
                        )}
                        <div>
                            <Action
                                label={job.cancelRequested ? words.cancelling : words.cancel}
                                icon={X}
                                pending={cancel.isPending || job.cancelRequested}
                                reason={cancelReason}
                                onPress={() => cancel.mutate()}
                                onBlocked={(reason) => onOutcome({ tone: 'warning', message: reason })}
                            />
                        </div>
                    </>
                )}
                {job.state === 'installed' && (
                    <div className="flex flex-wrap items-center gap-3">
                        <p className="text-body text-success">{words.installed}</p>
                        {canManage && (
                            <Button size="sm" onClick={() => onUse(job.build)}>
                                {words.use}
                            </Button>
                        )}
                    </div>
                )}
                {job.state === 'failed' && job.failure !== null && (
                    <div className="flex flex-wrap items-center gap-3">
                        <p className="text-body text-danger-ink">
                            {words.failed(words.during[job.step], failureText(t, job.failure, format.bytes))}
                        </p>
                        {job.kind === 'install' && canManage && (
                            <Button size="sm" icon={RefreshCw} onClick={() => onRetry(job.build)}>
                                {words.retry}
                            </Button>
                        )}
                    </div>
                )}
                {job.state === 'cancelled' && <p className="text-body text-ink-2">{words.cancelled}</p>}
            </div>
        </Card>
    );
}

function chipsOf(
    t: ReturnType<typeof useI18n>['t'],
    status: ArtifactsStatus,
    build: InstalledBuild,
    entry: CatalogEntry | undefined,
) {
    const words = t.artifacts.installed.chips;
    const chips: { label: string; tone: 'neutral' | 'success' | 'warning' | 'danger' }[] = [];
    if (status.running === build.build) chips.push({ label: words.running, tone: 'success' });
    if (status.selected === build.build) chips.push({ label: words.nextStart, tone: 'neutral' });
    if (status.pending?.build === build.build) chips.push({ label: words.pending, tone: 'neutral' });
    if (status.lastHealthy?.build === build.build) chips.push({ label: words.lastHealthy, tone: 'neutral' });
    if (status.probation?.build === build.build) chips.push({ label: words.probation, tone: 'warning' });
    if (entry?.recommended) chips.push({ label: words.recommended, tone: 'neutral' });
    if (entry?.latest) chips.push({ label: words.latest, tone: 'neutral' });
    if (build.health.kind === 'broken') chips.push({ label: words.broken, tone: 'danger' });
    if (entry?.support === 'endOfLife') {
        chips.push({ label: words.endOfLife, tone: 'danger' });
    } else if (entry?.endOfSupport?.kind === 'at' && entry.support === 'endOfSupport') {
        chips.push({ label: words.endOfSupport(day(entry.endOfSupport.at)), tone: 'warning' });
    }
    return chips;
}

function InstalledCard({
    status,
    catalog,
    onOpen,
    onOutcome,
}: {
    status: ArtifactsStatus;
    catalog: CatalogView | null;
    onOpen: (open: Open) => void;
    onOutcome: (outcome: Outcome) => void;
}) {
    const { t, format } = useI18n();
    const me = useMe();
    const id = useId();
    const words = t.artifacts.installed;
    const canManage = allows(me, Permission.ArtifactsManage);
    const blocked = (reason: string) => onOutcome({ tone: 'warning', message: reason });
    return (
        <Card
            id={id}
            icon={HardDrive}
            title={words.title}
            flush
            aside={
                <Action
                    label={words.importBuild}
                    icon={FolderInput}
                    size="sm"
                    tone="ghost"
                    reason={me.isOwner ? null : words.ownerOnly}
                    onPress={() => onOpen({ kind: 'import' })}
                    onBlocked={blocked}
                />
            }
        >
            {status.installed.length === 0 ? (
                <p className="p-5 text-body text-ink-3 max-md:p-4">{words.empty}</p>
            ) : (
                <div className={table.scroll}>
                    <table className={table.table}>
                        <thead>
                            <tr>
                                <th className={table.th}>{words.build}</th>
                                <th className={table.th}>{words.state}</th>
                                <th className={table.th}>{words.installedAt}</th>
                                <th className={table.th}>{words.size}</th>
                                <th className={cn(table.th, 'text-right')}>{words.actions}</th>
                            </tr>
                        </thead>
                        <tbody>
                            {status.installed.map((build) => {
                                const entry = catalog?.builds.find((each) => each.build === build.build);
                                const protections = protectionsOf(status, build.build);
                                const deleteReason = !canManage
                                    ? words.notPermitted
                                    : protections[0] === undefined
                                      ? null
                                      : words.protectedBy[protections[0]];
                                const useReason = !canManage
                                    ? words.notPermitted
                                    : build.health.kind === 'broken'
                                      ? build.health.reason === 'archiveChanged'
                                          ? words.archiveChanged
                                          : words.brokenReason
                                      : status.selected === build.build && status.pending === null
                                        ? words.current
                                        : null;
                                return (
                                    <tr key={build.build} className={table.row}>
                                        <td className={table.td}>
                                            <div className={table.mono}>{build.build}</div>
                                            <div className={table.sub}>
                                                {build.commit === null ? (
                                                    build.origin === 'import' ? (
                                                        words.imported
                                                    ) : null
                                                ) : (
                                                    <button
                                                        type="button"
                                                        title={build.commit}
                                                        aria-label={words.copyCommit(build.commit)}
                                                        className="inline-flex items-center gap-1 font-mono hover:text-ink max-md:min-h-11"
                                                        onClick={() => void copyText(build.commit ?? '')}
                                                    >
                                                        {build.commit.slice(0, 7)}
                                                        <Copy
                                                            size={11}
                                                            strokeWidth={1.75}
                                                            aria-hidden="true"
                                                        />
                                                    </button>
                                                )}
                                            </div>
                                        </td>
                                        <td className={table.td}>
                                            <div className="flex flex-wrap gap-1.5">
                                                {chipsOf(t, status, build, entry).map((chip) => (
                                                    <Chip key={chip.label} tone={chip.tone}>
                                                        {chip.label}
                                                    </Chip>
                                                ))}
                                            </div>
                                        </td>
                                        <td className={cn(table.td, 'whitespace-nowrap tabular-nums')}>
                                            {build.installedAt === null ? '' : day(build.installedAt)}
                                            <div className={table.sub}>
                                                {build.installedBy ?? words.commandLine}
                                            </div>
                                        </td>
                                        <td className={cn(table.td, 'whitespace-nowrap tabular-nums')}>
                                            {format.bytes(build.bytes)}
                                        </td>
                                        <td className={cn(table.td, 'text-right')}>
                                            <div className="inline-flex flex-wrap justify-end gap-1.5">
                                                <Action
                                                    label={words.use}
                                                    size="sm"
                                                    reason={useReason}
                                                    onPress={() =>
                                                        onOpen({ kind: 'use', build: build.build })
                                                    }
                                                    onBlocked={blocked}
                                                />
                                                <Action
                                                    label={words.delete}
                                                    size="sm"
                                                    tone="ghost"
                                                    icon={Trash2}
                                                    reason={deleteReason}
                                                    onPress={() =>
                                                        onOpen({ kind: 'delete', build: build.build })
                                                    }
                                                    onBlocked={blocked}
                                                />
                                            </div>
                                        </td>
                                    </tr>
                                );
                            })}
                        </tbody>
                    </table>
                </div>
            )}
            <div className="flex flex-wrap items-center gap-3 border-t border-line px-5 py-3 max-md:px-4">
                <p className="mr-auto text-small text-ink-3">{words.keep(status.settings.keepBuilds)}</p>
                <Action
                    label={words.cleanup}
                    size="sm"
                    icon={Trash2}
                    reason={canManage ? null : words.notPermitted}
                    onPress={() => onOpen({ kind: 'cleanup' })}
                    onBlocked={blocked}
                />
            </div>
        </Card>
    );
}

type Filters = {
    recommended: boolean;
    latest: boolean;
    supportedOnly: boolean;
    endOfLife: boolean;
    notListed: boolean;
    search: string;
};

const visible = (entry: CatalogEntry, filters: Filters) => {
    if (
        (filters.recommended || filters.latest) &&
        !((filters.recommended && entry.recommended) || (filters.latest && entry.latest))
    ) {
        return false;
    }
    if (filters.notListed && entry.listed) {
        return false;
    }
    if (entry.support === 'endOfLife' && !filters.endOfLife) {
        return false;
    }
    if (filters.supportedOnly && entry.support !== 'supported' && entry.support !== 'endOfLife') {
        return false;
    }
    const search = filters.search.trim().toLowerCase();
    return search === '' || String(entry.build).includes(search) || (entry.commit ?? '').startsWith(search);
};

function AvailableCard({
    status,
    catalog,
    onInstall,
    onOutcome,
}: {
    status: ArtifactsStatus;
    catalog: UseQueryResult<CatalogView>;
    onInstall: (entry: CatalogEntry) => void;
    onOutcome: (outcome: Outcome) => void;
}) {
    const { t, format } = useI18n();
    const me = useMe();
    const id = useId();
    const queryClient = useQueryClient();
    const zone = useZone();
    const words = t.artifacts.available;
    const [filters, setFilters] = useState<Filters>({
        recommended: false,
        latest: false,
        supportedOnly: true,
        endOfLife: false,
        notListed: false,
        search: '',
    });
    const refresh = useMutation({
        mutationFn: refreshCatalog,
        onSuccess: (view) => queryClient.setQueryData(catalogQuery.queryKey, view),
        onError: (error) =>
            onOutcome({ tone: 'danger', message: t.artifacts.failed(words.refresh, errorMessage(t, error)) }),
    });
    const canManage = allows(me, Permission.ArtifactsManage);
    const jobRunning = status.job?.state === 'running';
    const toggle = (name: keyof Omit<Filters, 'search'>, label: string) => (
        <label
            htmlFor={`${id}-${name}`}
            className="inline-flex cursor-pointer items-center gap-2 text-ui text-ink-2 max-md:min-h-11"
        >
            <Checkbox
                id={`${id}-${name}`}
                checked={filters[name]}
                onChange={(event) => setFilters({ ...filters, [name]: event.target.checked })}
            />
            {label}
        </label>
    );
    const view = catalog.data;
    const entries = view?.builds.filter((entry) => visible(entry, filters)) ?? [];
    return (
        <Card
            id={id}
            icon={Download}
            title={words.title}
            flush
            aside={
                <>
                    {view !== undefined && (
                        <span className="text-meta text-ink-3">
                            {words.listed(zone.when(view.fetchedAt))}
                        </span>
                    )}
                    <Button
                        size="sm"
                        tone="ghost"
                        icon={RefreshCw}
                        pending={refresh.isPending}
                        onClick={() => refresh.mutate()}
                    >
                        {words.refresh}
                    </Button>
                </>
            }
        >
            <div className="flex flex-col gap-3 border-b border-line px-5 py-3.5 max-md:px-4">
                <fieldset className="flex flex-wrap gap-x-5 gap-y-2">
                    <legend className="sr-only">{words.filters}</legend>
                    {toggle('recommended', words.recommended)}
                    {toggle('latest', words.latest)}
                    {toggle('supportedOnly', words.supportedOnly)}
                    {toggle('endOfLife', words.showEndOfLife)}
                    {toggle('notListed', words.notListed)}
                </fieldset>
                <label className="relative block max-w-sm">
                    <span className="sr-only">{words.search}</span>
                    <Search
                        size={14}
                        strokeWidth={1.5}
                        className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-ink-3"
                        aria-hidden="true"
                    />
                    <input
                        type="search"
                        className={cn(inputClass, 'pl-8.5')}
                        placeholder={words.search}
                        value={filters.search}
                        onChange={(event) => setFilters({ ...filters, search: event.target.value })}
                    />
                </label>
            </div>
            {catalog.isPending ? (
                <div className="flex flex-col gap-3 p-5" aria-busy="true">
                    <span className="sr-only">{words.loading}</span>
                    <span className="skeleton w-1/2" />
                    <span className="skeleton w-1/3" />
                </div>
            ) : catalog.isError ? (
                <p className="p-5 text-body text-danger-ink max-md:p-4">
                    {words.failed} {errorMessage(t, catalog.error)}
                </p>
            ) : entries.length === 0 ? (
                <p className="p-5 text-body text-ink-3 max-md:p-4">{words.none}</p>
            ) : (
                <div className={table.scroll}>
                    <table className={table.table}>
                        <thead>
                            <tr>
                                <th className={table.th}>{words.build}</th>
                                <th className={table.th}>{words.channel}</th>
                                <th className={table.th}>{words.published}</th>
                                <th className={table.th}>{words.support}</th>
                                <th className={table.th}>{words.size}</th>
                                <th className={cn(table.th, 'text-right')}>{words.action}</th>
                            </tr>
                        </thead>
                        <tbody>
                            {entries.map((entry) => {
                                const reason = entry.installed
                                    ? words.isInstalled
                                    : entry.availability.kind === 'noArchive'
                                      ? words.noArchive(words.platforms[view?.platform ?? 'windows'])
                                      : entry.availability.kind === 'noCommit'
                                        ? words.noCommit
                                        : !canManage
                                          ? t.artifacts.installed.notPermitted
                                          : jobRunning
                                            ? t.errors.job_running
                                            : null;
                                const supportEnds = policyDay(entry.endOfSupport, words.never);
                                const lifeEnds = policyDay(entry.endOfLife, words.never);
                                return (
                                    <tr key={entry.build} className={table.row}>
                                        <td className={table.td}>
                                            <div className={table.mono}>{entry.build}</div>
                                            {entry.commit !== null && (
                                                <div
                                                    className={cn(table.sub, 'font-mono')}
                                                    title={entry.commit}
                                                >
                                                    {entry.commit.slice(0, 7)}
                                                </div>
                                            )}
                                        </td>
                                        <td className={table.td}>
                                            <div className="flex flex-wrap gap-1.5">
                                                {entry.recommended && <Chip>{words.recommended}</Chip>}
                                                {entry.latest && <Chip>{words.latest}</Chip>}
                                                {!entry.recommended &&
                                                    !entry.latest &&
                                                    entry.branch !== null && (
                                                        <span className="font-mono text-small">
                                                            {entry.branch}
                                                        </span>
                                                    )}
                                            </div>
                                        </td>
                                        <td className={cn(table.td, 'whitespace-nowrap tabular-nums')}>
                                            {entry.publishedAt === null
                                                ? words.unknownDate
                                                : day(entry.publishedAt)}
                                        </td>
                                        <td className={table.td}>
                                            <Chip
                                                tone={
                                                    entry.support === 'endOfLife'
                                                        ? 'danger'
                                                        : entry.support === 'endOfSupport'
                                                          ? 'warning'
                                                          : entry.support === 'supported'
                                                            ? 'success'
                                                            : 'neutral'
                                                }
                                            >
                                                {words.supports[entry.support]}
                                            </Chip>
                                            <div className={table.sub}>
                                                {supportEnds !== null && (
                                                    <span>{words.supportEnds(supportEnds)}</span>
                                                )}
                                                {lifeEnds !== null && <span>{words.lifeEnds(lifeEnds)}</span>}
                                            </div>
                                        </td>
                                        <td className={cn(table.td, 'whitespace-nowrap tabular-nums')}>
                                            {entry.availability.kind === 'downloadable' &&
                                            entry.availability.size !== null
                                                ? format.bytes(entry.availability.size)
                                                : ''}
                                        </td>
                                        <td className={cn(table.td, 'text-right')}>
                                            {entry.installed || entry.availability.kind !== 'downloadable' ? (
                                                <span className="text-small text-ink-3">{reason}</span>
                                            ) : (
                                                <Action
                                                    label={words.install}
                                                    size="sm"
                                                    reason={reason}
                                                    onPress={() => onInstall(entry)}
                                                    onBlocked={(blocked) =>
                                                        onOutcome({ tone: 'warning', message: blocked })
                                                    }
                                                />
                                            )}
                                        </td>
                                    </tr>
                                );
                            })}
                        </tbody>
                    </table>
                </div>
            )}
        </Card>
    );
}

function SettingsCard({ status }: { status: ArtifactsStatus }) {
    const { t } = useI18n();
    const me = useMe();
    const id = useId();
    const words = t.artifacts.settings;
    const canManage = allows(me, Permission.ArtifactsManage);
    const [shown, setShown] = useState({ settings: status.settings, version: status.version });
    const [draft, setDraft] = useState<ArtifactSettings>(status.settings);
    const [branch, setBranch] = useState('');
    const [issues, setIssues] = useState<ArtifactSettingsIssue[]>([]);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const changes = (Object.keys(draft) as (keyof ArtifactSettings)[]).filter(
        (key) => JSON.stringify(draft[key]) !== JSON.stringify(shown.settings[key]),
    ).length;
    const stale = status.version !== shown.version;
    if (stale && changes === 0) {
        setShown({ settings: status.settings, version: status.version });
        setDraft(status.settings);
    }
    const save = useMutation({
        mutationFn: () => saveArtifactSettings({ settings: draft, version: shown.version }),
        onMutate: () => {
            setOutcome(null);
            setIssues([]);
        },
        onSuccess: (saved) => {
            setShown({ settings: saved.settings, version: saved.version });
            setDraft(saved.settings);
            setOutcome({ tone: 'success', message: words.saved });
        },
        onError: (error) => {
            if (error instanceof ApiError && error.code === 'invalid_artifact_settings') {
                setIssues(settingsIssues(error.params));
            }
            setOutcome({ tone: 'danger', message: errorMessage(t, error) });
        },
    });
    const issueOf = (field: ArtifactSettingsIssue['field'], index: number | null = null) => {
        const found = issues.find((issue) => issue.field === field && issue.index === index);
        return found === undefined ? undefined : words.problems[found.problem];
    };
    const addBranch = () => {
        const name = branch.trim();
        if (name !== '' && !draft.branches.includes(name)) {
            setDraft({ ...draft, branches: [...draft.branches, name] });
            setBranch('');
        }
    };
    return (
        <form
            className="flex flex-col gap-3"
            noValidate
            onSubmit={(event) => {
                event.preventDefault();
                if (canManage && !save.isPending) {
                    save.mutate();
                }
            }}
        >
            {stale && changes > 0 && <Notice tone="warning">{words.changedElsewhere}</Notice>}
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            <Card id={id} icon={Settings2} title={words.title}>
                <fieldset disabled={!canManage} className="flex max-w-2xl min-w-0 flex-col gap-5">
                    {!canManage && <p className="text-small text-ink-3">{words.readOnly}</p>}
                    <TextField
                        label={words.keepBuilds}
                        hint={words.keepBuildsHint}
                        type="number"
                        inputMode="numeric"
                        min={1}
                        max={20}
                        className="max-w-48"
                        value={String(draft.keepBuilds)}
                        error={issueOf('keepBuilds')}
                        onChange={(event) => setDraft({ ...draft, keepBuilds: Number(event.target.value) })}
                    />
                    <label
                        htmlFor={`${id}-rollback`}
                        className="-mx-2 flex cursor-pointer items-start gap-3 rounded-sm px-2 py-2 hover:bg-hover"
                    >
                        <Checkbox
                            id={`${id}-rollback`}
                            checked={draft.autoRollback}
                            onChange={(event) => setDraft({ ...draft, autoRollback: event.target.checked })}
                        />
                        <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                            <span className="text-body font-medium text-ink">{words.autoRollback}</span>
                            <span className="text-small text-ink-2">{words.autoRollbackHint}</span>
                        </span>
                    </label>
                    <div className="flex flex-col gap-2">
                        <p className="text-ui font-medium text-ink">{words.branches}</p>
                        <p className="text-small text-ink-3">{words.branchesHint}</p>
                        <ul className="flex flex-wrap gap-1.5">
                            {draft.branches.length === 0 && (
                                <li className="text-small text-ink-3">{words.noBranch}</li>
                            )}
                            {draft.branches.map((name, index) => (
                                <li key={name} className="flex flex-col gap-1">
                                    <span className="inline-flex h-7 items-center gap-1 rounded-pill border border-line bg-surface-2 pr-1 pl-2.5 font-mono text-small text-ink-2">
                                        {name}
                                        <button
                                            type="button"
                                            className="grid size-5 place-items-center rounded-pill hover:bg-hover max-md:size-11"
                                            aria-label={words.removeBranch(name)}
                                            onClick={() =>
                                                setDraft({
                                                    ...draft,
                                                    branches: draft.branches.filter((_, at) => at !== index),
                                                })
                                            }
                                        >
                                            <X size={12} strokeWidth={2} aria-hidden="true" />
                                        </button>
                                    </span>
                                    {issueOf('branches', index) && (
                                        <FieldError>{issueOf('branches', index)}</FieldError>
                                    )}
                                </li>
                            ))}
                        </ul>
                        {issueOf('branches') && <FieldError>{issueOf('branches')}</FieldError>}
                        <div className="flex max-w-md flex-wrap gap-2">
                            <input
                                aria-label={words.branchName}
                                className={cn(inputClass, 'min-w-0 flex-1 font-mono text-ui')}
                                value={branch}
                                spellCheck={false}
                                autoCapitalize="none"
                                onChange={(event) => setBranch(event.target.value)}
                                onKeyDown={(event) => {
                                    if (event.key === 'Enter') {
                                        event.preventDefault();
                                        addBranch();
                                    }
                                }}
                            />
                            <Button icon={Plus} onClick={addBranch} disabled={draft.branches.length >= 10}>
                                {words.addBranch}
                            </Button>
                        </div>
                    </div>
                </fieldset>
            </Card>
            {canManage && changes > 0 && (
                <div className="sticky bottom-3 z-10 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-box border border-line-strong bg-surface px-3.5 py-2.5 shadow-pop">
                    <output className="mr-auto text-ui text-ink-2">{words.changes(changes)}</output>
                    <Button
                        tone="ghost"
                        className="max-md:flex-1"
                        onClick={() => {
                            setShown({ settings: status.settings, version: status.version });
                            setDraft(status.settings);
                            setIssues([]);
                            setOutcome({ tone: 'success', message: words.discarded });
                        }}
                    >
                        {words.discard}
                    </Button>
                    <Button type="submit" tone="ink" pending={save.isPending} className="max-md:flex-1">
                        {words.save}
                    </Button>
                </div>
            )}
        </form>
    );
}

function Detail({ label, value }: { label: string; value: ReactNode }) {
    return (
        <div className="flex justify-between gap-4 border-b border-line py-1.5 last:border-b-0">
            <dt className="text-ink-3">{label}</dt>
            <dd className="text-right text-ink">{value}</dd>
        </div>
    );
}

function Allowance({
    label,
    checked,
    onChange,
}: {
    label: string;
    checked: boolean;
    onChange: (checked: boolean) => void;
}) {
    const id = useId();
    return (
        <label
            htmlFor={id}
            className="flex cursor-pointer items-start gap-3 rounded-sm text-small text-danger-ink"
        >
            <Checkbox id={id} checked={checked} onChange={(event) => onChange(event.target.checked)} />
            <span>{label}</span>
        </label>
    );
}

export function InstallDialog({
    entry,
    status,
    onClose,
    onDone,
}: {
    entry: CatalogEntry;
    status: ArtifactsStatus;
    onClose: () => void;
    onDone: (message: string) => void;
}) {
    const { t, format } = useI18n();
    const words = t.artifacts.install;
    const available = t.artifacts.available;
    const endOfLife = entry.support === 'endOfLife';
    const [allowed, setAllowed] = useState(false);
    const install = useMutation({
        mutationFn: () => installBuild(entry.build, endOfLife && allowed),
        onSuccess: () => onDone(available.started(String(entry.build))),
    });
    const blocked = endOfLife && !allowed;
    return (
        <Dialog
            title={words.title(String(entry.build))}
            onClose={onClose}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.artifacts.cancel}
                    </Button>
                    <Button
                        tone="ink"
                        icon={Download}
                        pending={install.isPending}
                        disabled={blocked}
                        onClick={() => install.mutate()}
                    >
                        {words.confirm}
                    </Button>
                </>
            }
        >
            <dl className="text-small">
                <Detail
                    label={words.channel}
                    value={[
                        entry.recommended && available.recommended,
                        entry.latest && available.latest,
                        entry.branch,
                    ]
                        .filter(Boolean)
                        .join(' · ')}
                />
                <Detail
                    label={words.published}
                    value={entry.publishedAt === null ? available.unknownDate : day(entry.publishedAt)}
                />
                <Detail label={words.support} value={available.supports[entry.support]} />
                {entry.availability.kind === 'downloadable' && entry.availability.size !== null && (
                    <Detail label={words.size} value={format.bytes(entry.availability.size)} />
                )}
                {status.disk !== null && <Detail label={words.free} value={format.bytes(status.disk.free)} />}
            </dl>
            {status.longPath !== null && (
                <Notice tone="warning">{t.artifacts.notices.longPath(format.number(status.longPath))}</Notice>
            )}
            {endOfLife && <Allowance label={words.endOfLife} checked={allowed} onChange={setAllowed} />}
            {install.isError && <Notice tone="danger">{errorMessage(t, install.error)}</Notice>}
        </Dialog>
    );
}

export function UseDialog({
    build,
    status,
    catalog,
    onClose,
    onDone,
}: {
    build: number;
    status: ArtifactsStatus;
    catalog: CatalogView | null;
    onClose: () => void;
    onDone: (message: string) => void;
}) {
    const { t } = useI18n();
    const me = useMe();
    const { status: server } = useServerStatus();
    const { schedule } = useSchedule();
    const zone = useZone();
    const words = t.artifacts.use;
    const running = server !== null && isRunning(server.state);
    const next = schedule?.next ?? null;
    const [mode, setMode] = useState<ApplyMode>(() => defaultMode(running, next !== null));
    const [typed, setTyped] = useState('');
    const [allowed, setAllowed] = useState(false);
    const kvp = needsKvpConfirmation(status, build, mode);
    const endOfLife = catalog?.builds.find((entry) => entry.build === build)?.support === 'endOfLife';
    const nowReason = !allows(me, Permission.ServerControl)
        ? words.notPermitted
        : server?.state === 'stopping'
          ? words.stopping
          : !running
            ? words.notRunning
            : null;
    const scheduledReason = next === null ? words.noRestart : null;
    const select = useMutation({
        mutationFn: () =>
            selectBuild({
                build,
                apply: mode,
                confirmKvpRollback: kvp && typed.trim() === String(KVP_BUILD),
                allowEndOfLife: endOfLife && allowed,
            }),
        onSuccess: () =>
            onDone(
                mode === 'now' && status.running === build
                    ? words.kept(String(build))
                    : words.done[mode](String(build)),
            ),
    });
    const blocked =
        (kvp && typed.trim() !== String(KVP_BUILD)) ||
        (endOfLife && !allowed) ||
        (mode === 'now' && nowReason !== null) ||
        (mode === 'scheduledRestart' && scheduledReason !== null);
    const option = (value: ApplyMode, label: string, hint: string | null, reason: string | null) => (
        <label
            className={cn(
                'flex items-start gap-3 rounded-box border border-line px-3.5 py-3 has-checked:border-accent has-checked:bg-surface-2',
                reason === null ? 'cursor-pointer' : 'cursor-not-allowed opacity-60',
            )}
        >
            <input
                type="radio"
                name="apply"
                value={value}
                className="mt-1 accent-[var(--accent)]"
                checked={mode === value}
                disabled={reason !== null}
                onChange={() => setMode(value)}
            />
            <span className="flex min-w-0 flex-col gap-0.5">
                <span className="text-body font-medium text-ink">{label}</span>
                {(reason ?? hint) !== null && <span className="text-small text-ink-2">{reason ?? hint}</span>}
            </span>
        </label>
    );
    return (
        <Dialog
            title={words.title(String(build))}
            onClose={onClose}
            onSubmit={() => {
                if (!blocked && !select.isPending) {
                    select.mutate();
                }
            }}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.artifacts.cancel}
                    </Button>
                    <Button type="submit" tone="ink" pending={select.isPending} disabled={blocked}>
                        {words.confirm}
                    </Button>
                </>
            }
        >
            <fieldset className="flex flex-col gap-2">
                <legend className="mb-2 text-ui font-medium text-ink">{words.when}</legend>
                {option(
                    'now',
                    words.now,
                    server?.players == null ? null : words.nowPlayers(server.players.online),
                    nowReason,
                )}
                {option(
                    'nextStart',
                    words.nextStart,
                    running && status.running !== null
                        ? words.nextStartRunning(String(status.running))
                        : words.nextStartStopped,
                    null,
                )}
                {option(
                    'scheduledRestart',
                    words.scheduled,
                    next === null || schedule === null ? null : `${zone.when(next.at)} ${schedule.zone}`,
                    scheduledReason,
                )}
            </fieldset>
            {mode === 'scheduledRestart' && status.pending !== null && status.pending.build !== build && (
                <p className="text-small text-ink-2">{words.replaces(String(status.pending.build))}</p>
            )}
            {kvp && (
                <div className="flex flex-col gap-2">
                    <p className="text-small text-danger-ink">{words.kvp}</p>
                    <TextField
                        label={words.kvpLabel}
                        inputMode="numeric"
                        autoComplete="off"
                        value={typed}
                        onChange={(event) => setTyped(event.target.value)}
                    />
                </div>
            )}
            {endOfLife && <Allowance label={words.endOfLife} checked={allowed} onChange={setAllowed} />}
            {select.isError && <Notice tone="danger">{errorMessage(t, select.error)}</Notice>}
        </Dialog>
    );
}

export function DeleteDialog({
    build,
    status,
    catalog,
    onClose,
    onDone,
}: {
    build: number;
    status: ArtifactsStatus;
    catalog: CatalogView | null;
    onClose: () => void;
    onDone: (message: string) => void;
}) {
    const { t, format } = useI18n();
    const words = t.artifacts.delete;
    const installed = status.installed.find((each) => each.build === build);
    const entry = catalog?.builds.find((each) => each.build === build);
    const reinstallable = entry !== undefined && entry.availability.kind === 'downloadable';
    const remove = useMutation({
        mutationFn: () => deleteBuild(build),
        onSuccess: () => onDone(t.artifacts.installed.deleted(String(build))),
    });
    return (
        <Dialog
            title={words.title(String(build))}
            role="alertdialog"
            onClose={onClose}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.artifacts.cancel}
                    </Button>
                    <Button
                        tone="danger"
                        icon={Trash2}
                        pending={remove.isPending}
                        onClick={() => remove.mutate()}
                    >
                        {words.confirm}
                    </Button>
                </>
            }
        >
            <p>
                {words.frees(format.bytes(installed?.bytes ?? 0))}{' '}
                {catalog !== null && !reinstallable && words.gone}
            </p>
            {remove.isError && <Notice tone="danger">{errorMessage(t, remove.error)}</Notice>}
        </Dialog>
    );
}

export function CleanupDialog({
    onClose,
    onDone,
}: {
    onClose: () => void;
    onDone: (message: string) => void;
}) {
    const { t, format } = useI18n();
    const words = t.artifacts.cleanup;
    const preview = useQuery(cleanupQuery);
    const queryClient = useQueryClient();
    const clean = useMutation({
        mutationFn: () => cleanUp(preview.data?.remove.map((each) => each.build) ?? []),
        onSuccess: (result) => {
            const skipped = result.skipped.map((each) => String(each.build)).join(', ');
            onDone(
                [
                    words.done(result.deleted.length, format.bytes(result.freed)),
                    skipped === '' ? null : words.skipped(skipped),
                ]
                    .filter(Boolean)
                    .join(' '),
            );
        },
        onError: () => void queryClient.invalidateQueries({ queryKey: cleanupQuery.queryKey }),
    });
    const remove = preview.data?.remove ?? [];
    return (
        <Dialog
            title={words.title}
            role="alertdialog"
            onClose={onClose}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.artifacts.cancel}
                    </Button>
                    <Button
                        tone="danger"
                        icon={Trash2}
                        pending={clean.isPending}
                        disabled={preview.data === undefined || remove.length === 0}
                        onClick={() => clean.mutate()}
                    >
                        {words.confirm(remove.length)}
                    </Button>
                </>
            }
        >
            {preview.isPending ? (
                <p className="text-ink-3" aria-busy="true">
                    {words.loading}
                </p>
            ) : preview.isError ? (
                <Notice tone="danger">{errorMessage(t, preview.error)}</Notice>
            ) : remove.length === 0 ? (
                <p>{words.none}</p>
            ) : (
                <>
                    <ul className="flex flex-col gap-1 text-small">
                        {remove.map((each) => (
                            <li
                                key={each.build}
                                className="flex justify-between gap-4 border-b border-line py-1.5 last:border-b-0"
                            >
                                <span className="font-mono text-ink">{each.build}</span>
                                <span className="text-ink-3">
                                    {format.bytes(each.bytes)}
                                    {!each.reinstallable && ` · ${words.gone}`}
                                </span>
                            </li>
                        ))}
                    </ul>
                    <p className="font-medium text-ink">{words.freed(format.bytes(preview.data.freed))}</p>
                </>
            )}
            {clean.isError && <Notice tone="danger">{errorMessage(t, clean.error)}</Notice>}
        </Dialog>
    );
}

export function ImportDialog({
    onClose,
    onDone,
}: {
    onClose: () => void;
    onDone: (message: string) => void;
}) {
    const { t } = useI18n();
    const words = t.artifacts.import;
    const me = useMe();
    // The folder FXServer ran from before is the owner's to read, in the launch settings.
    const launch = useQuery({ ...launchQuery, enabled: me.isOwner });
    const legacy = launch.data?.legacyBuildPath ?? null;
    const [folder, setFolder] = useState(legacy ?? '');
    const [prefilled, setPrefilled] = useState(legacy !== null);
    if (!prefilled && legacy !== null) {
        setPrefilled(true);
        setFolder((current) => (current === '' ? legacy : current));
    }
    const [build, setBuild] = useState('');
    const [touched, setTouched] = useState(false);
    const valid = /^\d{4,7}$/.test(build.trim());
    const start = useMutation({
        mutationFn: () => importBuild(folder.trim(), Number(build.trim())),
        onSuccess: () => onDone(words.started(build.trim())),
    });
    const refused =
        start.error instanceof ApiError && start.error.code === 'invalid_import_folder'
            ? launchProblem(start.error.params.problem)
            : null;
    return (
        <Dialog
            title={words.title}
            onClose={onClose}
            closeOnBackdrop={folder === '' && build === ''}
            onSubmit={() => {
                setTouched(true);
                if (valid && folder.trim() !== '' && !start.isPending) {
                    start.mutate();
                }
            }}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.artifacts.cancel}
                    </Button>
                    <Button type="submit" tone="ink" icon={FolderInput} pending={start.isPending}>
                        {words.confirm}
                    </Button>
                </>
            }
        >
            <p>{words.about}</p>
            <TextField
                label={words.folder}
                hint={words.folderHint}
                value={folder}
                spellCheck={false}
                autoCapitalize="none"
                autoComplete="off"
                inputClassName="font-mono text-ui"
                error={
                    touched && folder.trim() === ''
                        ? t.validation.required
                        : refused === null
                          ? undefined
                          : t.server.problems[refused]
                }
                onChange={(event) => {
                    setFolder(event.target.value);
                    start.reset();
                }}
            />
            <TextField
                label={words.build}
                hint={words.buildHint}
                value={build}
                inputMode="numeric"
                autoComplete="off"
                inputClassName="font-mono"
                error={touched && !valid ? words.invalidBuild : undefined}
                onChange={(event) => setBuild(event.target.value)}
            />
            {start.isError && refused === null && (
                <Notice tone="danger">{errorMessage(t, start.error)}</Notice>
            )}
        </Dialog>
    );
}
