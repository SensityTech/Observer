import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Download, FolderInput } from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode, type Ref } from 'react';

import { catalogQuery } from '../api/artifacts';
import type { CatalogEntry } from '../api/generated/CatalogEntry';
import { useArtifacts } from '../artifacts/context';
import { Button } from '../components/Button';
import { Notice } from '../components/Notice';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import { ImportDialog, InstallDialog, JobCard, UseDialog, type Outcome } from '../pages/ArtifactsPage';
import { StepActions } from './StepActions';
import { StepPage } from './StepPage';

type Open = { kind: 'install'; entry: CatalogEntry } | { kind: 'import' } | { kind: 'use'; build: number };

/**
 * The build FXServer starts, installed or imported here with the Artifacts page's own dialogs and
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
    // A job that ended before the step opened is the Artifacts page's to show, not this setup's.
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

    const recommended = catalog.data?.builds.find((entry) => entry.recommended) ?? null;
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
    const usable = status?.installed.filter((build) => build.health.kind === 'valid') ?? [];
    const job = status?.job ?? null;

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
                <div className="flex max-w-200 flex-col gap-5">
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
                        <div className="flex flex-col gap-4">
                            {(selected === null || outcome === null) && (
                                <p className="text-body font-medium text-ink">
                                    {selected === null ? words.none : words.selected(String(selected))}
                                </p>
                            )}
                            {selected === null && (
                                <>
                                    <div className="flex flex-col gap-2">
                                        <div className="flex flex-wrap gap-2 max-md:flex-col max-md:items-stretch">
                                            <Button
                                                icon={Download}
                                                disabled={installReason !== null}
                                                onClick={() => {
                                                    if (recommended !== null) {
                                                        setOpen({ kind: 'install', entry: recommended });
                                                    }
                                                }}
                                            >
                                                {words.install}
                                            </Button>
                                            <Button
                                                icon={FolderInput}
                                                onClick={() => setOpen({ kind: 'import' })}
                                            >
                                                {words.importFolder}
                                            </Button>
                                        </div>
                                        {installReason !== null && (
                                            <p className="text-small text-ink-3">{installReason}</p>
                                        )}
                                    </div>
                                    {usable.length > 0 && (
                                        <div className="flex flex-col gap-1.5">
                                            <p className="text-ui font-medium text-ink">{words.installed}</p>
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
                                        </div>
                                    )}
                                </>
                            )}
                            <Link
                                to="/artifacts"
                                className="inline-flex items-center gap-1.5 self-start text-ui font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                            >
                                {words.openArtifacts}
                                <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                            </Link>
                        </div>
                    )}
                    {job !== null && job.id !== earlierJob && (
                        <JobCard
                            job={job}
                            startsNext={selected === job.build && status?.pending === null}
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
