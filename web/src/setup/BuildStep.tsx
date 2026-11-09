import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { Check, Download, FolderInput, Info, Package, type LucideIcon } from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode, type Ref } from 'react';

import { catalogQuery } from '../api/artifacts';
import type { CatalogEntry } from '../api/generated/CatalogEntry';
import { useArtifacts } from '../artifacts/context';
import { supportTone } from '../artifacts/support';
import { Button, buttonVariants } from '../components/Button';
import { Chip } from '../components/Chip';
import { Notice } from '../components/Notice';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useLinkState } from '../live/context';
import { ImportDialog, InstallDialog, JobCard, UseDialog, type Outcome } from '../pages/ArtifactsPage';
import { StepActions } from './StepActions';
import { StepPage } from './StepPage';

type Open = { kind: 'install'; entry: CatalogEntry } | { kind: 'import' } | { kind: 'use'; build: number };

/**
 * The build FXServer starts: the one selected, or the one Cfx.re recommends to install, with the
 * other ways to get one. Installs and imports run with the FXServer builds page's own dialogs and
 * job card. The step may be skipped: the settings save without a build, only FXServer cannot start.
 */
export function BuildStep({
    titleRef,
    banner,
    onBack,
    onContinue,
}: {
    titleRef: Ref<HTMLHeadingElement>;
    banner?: ReactNode;
    onBack: () => void;
    onContinue: () => void;
}) {
    const { t } = useI18n();
    const words = t.serverSetup.build;
    const link = useLinkState();
    const { status } = useArtifacts();
    const catalog = useQuery(catalogQuery);
    const [open, setOpen] = useState<Open | null>(null);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const notice = useRef<HTMLDivElement>(null);
    // A job that ended before the step opened is the FXServer builds page's to show, not this setup's.
    const [earlierJob] = useState(() =>
        status?.job?.state === 'running' ? null : (status?.job?.id ?? null),
    );
    if (outcome?.until !== undefined && status !== null && outcome.until(status)) {
        setOutcome(null);
    }
    // A dialog closed by its own button leaves the keyboard nowhere, its trigger gone with the
    // choice it offered: the keyboard goes to the notice saying what came of it.
    useEffect(() => {
        if (outcome !== null && document.activeElement === document.body) {
            notice.current?.focus();
        }
    }, [outcome]);
    const close = () => setOpen(null);
    const done = (message: string, until?: Outcome['until']) => {
        setOpen(null);
        setOutcome({ tone: 'success', message, until });
    };

    const builds = catalog.data?.builds ?? [];
    const recommended = builds.find((entry) => entry.recommended) ?? null;
    const latest = builds.find((entry) => entry.latest) ?? null;
    const installReason = catalog.isPending
        ? words.catalogLoading
        : catalog.isError
          ? words.catalogFailed
          : recommended === null || recommended.availability.kind !== 'downloadable'
            ? words.noRecommended
            : status?.installed.some((build) => build.build === recommended.build)
              ? words.recommendedInstalled(String(recommended.build))
              : status?.job?.state === 'running'
                ? t.errors.job_running
                : null;
    const selected = status?.selected ?? null;
    const usable =
        status?.installed.filter((build) => build.health.kind === 'valid' && build.build !== selected) ?? [];
    const job = status?.job ?? null;
    const shownJob = job !== null && job.id !== earlierJob ? job : null;

    return (
        <>
            <StepPage
                step="build"
                titleRef={titleRef}
                title={t.serverSetup.headings.build}
                lead={words.lead}
                banner={banner}
                actions={
                    <StepActions onBack={onBack}>
                        {selected === null && (
                            <Button tone="ghost" onClick={onContinue}>
                                {words.skip}
                            </Button>
                        )}
                        <Button tone="ink" disabled={selected === null} onClick={onContinue}>
                            {t.serverSetup.continue}
                        </Button>
                    </StepActions>
                }
            >
                <div className="flex max-w-250 flex-col gap-6">
                    {outcome !== null && (
                        <Notice ref={notice} tabIndex={-1} tone={outcome.tone}>
                            {outcome.message}
                        </Notice>
                    )}
                    {status === null ? (
                        <p
                            className="text-body text-ink-3"
                            aria-busy={link === 'live' || link === 'connecting'}
                        >
                            {link === 'live' || link === 'connecting' ? t.artifacts.loading : words.unknown}
                        </p>
                    ) : (
                        <>
                            {selected !== null ? (
                                <BuildCard
                                    label={words.selectedBuild}
                                    build={selected}
                                    entry={builds.find((entry) => entry.build === selected) ?? null}
                                    detail={words.startsFromIt}
                                    chosen
                                />
                            ) : recommended !== null ? (
                                <BuildCard
                                    label={words.recommendedBuild}
                                    build={recommended.build}
                                    entry={recommended}
                                    detail={words.recommendedFor(
                                        t.artifacts.available.platforms[status.platform],
                                    )}
                                    note={installReason}
                                    action={
                                        installReason === null && (
                                            <Button
                                                tone="ink"
                                                icon={Download}
                                                onClick={() =>
                                                    setOpen({ kind: 'install', entry: recommended })
                                                }
                                            >
                                                {words.install}
                                            </Button>
                                        )
                                    }
                                />
                            ) : (
                                <section className="card flex items-center gap-3.5 p-5 max-md:p-4">
                                    <span className="grid size-11 flex-none place-items-center rounded-control bg-surface-2 text-ink-2">
                                        <Package size={22} strokeWidth={1.5} aria-hidden="true" />
                                    </span>
                                    <span className="flex min-w-0 flex-col gap-1">
                                        <b className="text-card text-ink">{words.none}</b>
                                        <span className="text-small text-ink-3">{installReason}</span>
                                    </span>
                                </section>
                            )}
                            {shownJob !== null && (
                                <JobCard
                                    job={shownJob}
                                    startsNext={selected === shownJob.build && status.pending === null}
                                    onOutcome={setOutcome}
                                    onUse={(build) => setOpen({ kind: 'use', build })}
                                    onRetry={(build) => {
                                        const entry = builds.find((each) => each.build === build);
                                        if (entry !== undefined) {
                                            setOpen({ kind: 'install', entry });
                                        }
                                    }}
                                />
                            )}
                            {shownJob?.state === 'running' && (
                                <p className="flex items-center gap-2 text-small text-ink-3">
                                    <Info
                                        size={14}
                                        strokeWidth={1.5}
                                        className="flex-none"
                                        aria-hidden="true"
                                    />
                                    {words.goesOn}
                                </p>
                            )}
                            {usable.length > 0 && (
                                <section className="flex flex-col gap-3">
                                    <h2 className="eyebrow">{words.installed}</h2>
                                    <ul className="flex flex-wrap gap-2 max-md:flex-col max-md:items-stretch">
                                        {usable.map((build) => (
                                            <li key={build.build} className="flex flex-col">
                                                <Button
                                                    size="sm"
                                                    onClick={() =>
                                                        setOpen({ kind: 'use', build: build.build })
                                                    }
                                                >
                                                    {words.use(String(build.build))}
                                                </Button>
                                            </li>
                                        ))}
                                    </ul>
                                </section>
                            )}
                            <section className="flex flex-col gap-3">
                                <h2 className="eyebrow">
                                    {selected === null ? words.otherWays : words.change}
                                </h2>
                                <div className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,18rem),1fr))] gap-3">
                                    <Way
                                        icon={FolderInput}
                                        title={words.importTitle}
                                        detail={words.importDetail}
                                    >
                                        <Button size="sm" onClick={() => setOpen({ kind: 'import' })}>
                                            {words.choose}
                                        </Button>
                                    </Way>
                                    <Way
                                        icon={Package}
                                        title={words.anotherTitle}
                                        detail={
                                            latest !== null &&
                                            recommended !== null &&
                                            latest.build > recommended.build
                                                ? words.latestIs(String(latest.build))
                                                : words.anotherDetail
                                        }
                                    >
                                        <Link to="/artifacts" className={buttonVariants({ size: 'sm' })}>
                                            {words.seeAll}
                                        </Link>
                                    </Way>
                                </div>
                            </section>
                        </>
                    )}
                </div>
            </StepPage>
            {status !== null && open?.kind === 'install' && (
                <InstallDialog entry={open.entry} status={status} onClose={close} onDone={done} />
            )}
            {status !== null && open?.kind === 'use' && (
                <UseDialog
                    build={open.build}
                    status={status}
                    catalog={catalog.data ?? null}
                    onClose={close}
                    onDone={done}
                />
            )}
            {open?.kind === 'import' && <ImportDialog onClose={close} onDone={done} />}
        </>
    );
}

/** A build with what the catalog says of it: the one selected, or the one Cfx.re recommends. */
function BuildCard({
    label,
    build,
    entry,
    detail,
    chosen = false,
    note,
    action,
}: {
    label: string;
    build: number;
    entry: CatalogEntry | null;
    detail: string;
    chosen?: boolean;
    note?: string | null;
    action?: ReactNode;
}) {
    const { t } = useI18n();
    const words = t.serverSetup.build;
    const available = t.artifacts.available;
    return (
        <section aria-label={label} className="card flex flex-col gap-3 p-5 max-md:p-4">
            <div className="flex flex-wrap items-center gap-3.5">
                <span
                    className={cn(
                        'grid size-11 flex-none place-items-center rounded-control',
                        chosen ? 'bg-success-soft text-success' : 'bg-accent-soft text-accent-ink',
                    )}
                >
                    <Package size={22} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <span className="flex min-w-48 flex-1 flex-col gap-1">
                    <span className="flex flex-wrap items-center gap-2">
                        <span className="font-mono text-figure-sm text-ink">{build}</span>
                        {chosen && (
                            <Chip tone="success" icon={Check}>
                                {words.selected}
                            </Chip>
                        )}
                        {entry?.recommended === true && <Chip tone="accent">{available.recommended}</Chip>}
                        {entry !== null && entry.support !== 'unknown' && (
                            <Chip tone={supportTone(entry.support)}>{available.supports[entry.support]}</Chip>
                        )}
                    </span>
                    <span className="text-small text-ink-3">{detail}</span>
                </span>
                {action}
            </div>
            {note && <p className="text-small text-ink-3">{note}</p>}
        </section>
    );
}

/** Another way to get a build, with the button that takes it. */
function Way({
    icon: Icon,
    title,
    detail,
    children,
}: {
    icon: LucideIcon;
    title: string;
    detail: string;
    children: ReactNode;
}) {
    return (
        <div className="card flex items-center gap-3.5 p-4">
            <span className="grid size-9 flex-none place-items-center rounded-control bg-surface-2 text-ink-2">
                <Icon size={18} strokeWidth={1.5} aria-hidden="true" />
            </span>
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <b className="text-ui font-semibold text-ink">{title}</b>
                <span className="text-small text-ink-3">{detail}</span>
            </span>
            <span className="flex-none">{children}</span>
        </div>
    );
}
