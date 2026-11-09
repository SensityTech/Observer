import { useQuery } from '@tanstack/react-query';
import { ArrowLeft, Check, FolderOpen, LoaderCircle, RotateCw, Server, TriangleAlert } from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode, type Ref } from 'react';

import type { FolderCheck } from '../api/generated/FolderCheck';
import type { FolderReport } from '../api/generated/FolderReport';
import type { FoundServer } from '../api/generated/FoundServer';
import type { LaunchSettings } from '../api/generated/LaunchSettings';
import { discoveryQuery } from '../api/serverSetup';
import { Button } from '../components/Button';
import { Chip } from '../components/Chip';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { FolderBrowser } from './FolderBrowser';
import { FolderField } from './FolderField';
import { FolderVerdict } from './FolderVerdict';
import { crumbs, folderName } from './paths';
import type { Checked } from './state';
import { StepActions } from './StepActions';
import { StepPage } from './StepPage';

/**
 * The server folder: one of those Observer found on the host, a path typed with the host's folders
 * offered as it goes, or a folder picked in the browser. A server found is checked once picked and
 * the folder the step opens on at once, for the verdict to show before Continue; a path typed is
 * checked once chosen, and Continue checks the folder first when it was not.
 */
export function FolderStep({
    titleRef,
    banner,
    draft,
    checked,
    checking,
    onPath,
    onCheck,
    onContinue,
}: {
    titleRef: Ref<HTMLHeadingElement>;
    banner?: ReactNode;
    draft: LaunchSettings;
    checked: Checked<FolderCheck, FolderReport> | null;
    checking: boolean;
    onPath: (dataPath: string) => void;
    /** Checks a folder, and with `goOn` goes to the next step once it passes. */
    onCheck: (dataPath: string, goOn: boolean) => void;
    onContinue: () => void;
}) {
    const { t } = useI18n();
    const words = t.serverSetup.folder;
    const discovery = useQuery(discoveryQuery);
    const servers = discovery.data?.servers ?? [];
    const path = draft.dataPath.trim();
    const picked = servers.find((server) => server.path === path) ?? null;
    const [browsing, setBrowsing] = useState(false);
    const report = checked !== null && checked.request.dataPath === path ? checked.report : null;

    // With no folder yet, the first server found is offered as the one to run.
    const offered = useRef(false);
    useEffect(() => {
        const first = discovery.data?.servers[0];
        if (!offered.current && first !== undefined && draft.dataPath.trim() === '') {
            offered.current = true;
            onPath(first.path);
        }
    }, [discovery.data, draft.dataPath, onPath]);

    const [openedOn] = useState(path);
    const toCheck = picked?.path ?? (path !== '' && path === openedOn ? path : null);
    const checkedLast = useRef<string | null>(null);
    useEffect(() => {
        if (toCheck !== null && report === null && checkedLast.current !== toCheck) {
            checkedLast.current = toCheck;
            onCheck(toCheck, false);
        }
    }, [toCheck, report, onCheck, checkedLast]);

    const problem = report?.problem ?? null;
    const refusal =
        problem === null
            ? undefined
            : problem === 'invalidPath' && report?.pathProblem != null
              ? t.server.problems[report.pathProblem]
              : words.problems[problem];
    // A path typed has its refusal under the field: the verdict then has only a folder nearby to offer.
    const verdict = report !== null && (problem === null || picked !== null || report.suggestion !== null);
    const lead = discovery.isPending
        ? words.searching
        : discovery.isSuccess
          ? `${words.found(servers.length)}${discovery.data.truncated ? ` ${words.searchStopped}` : ''}`
          : null;
    const start =
        path !== ''
            ? (crumbs(path).at(-2)?.path ?? path)
            : ((servers[0] !== undefined ? crumbs(servers[0].path).at(-2)?.path : undefined) ??
              discovery.data?.places.find((place) => place.kind === 'home')?.path ??
              discovery.data?.places[0]?.path ??
              '/');

    return (
        <StepPage
            step="folder"
            titleRef={titleRef}
            title={t.serverSetup.headings.folder}
            lead={<output>{lead}</output>}
            banner={banner}
            actions={
                browsing ? (
                    // The browser's own button takes the folder it shows.
                    <StepActions>
                        <Button tone="ghost" icon={ArrowLeft} onClick={() => setBrowsing(false)}>
                            {words.browser.typeInstead}
                        </Button>
                    </StepActions>
                ) : (
                    <StepActions>
                        <Button
                            tone="ghost"
                            icon={RotateCw}
                            pending={discovery.isFetching}
                            onClick={() => void discovery.refetch()}
                        >
                            {words.rescan}
                        </Button>
                        <Button tone="ink" disabled={path === ''} pending={checking} onClick={onContinue}>
                            {picked === null
                                ? t.serverSetup.continue
                                : words.continueWith(folderName(picked.path))}
                        </Button>
                    </StepActions>
                )
            }
        >
            {browsing ? (
                <FolderBrowser
                    start={start}
                    // A path half typed is no folder to select.
                    chosen={picked?.path ?? (problem === null && report !== null ? path : null)}
                    places={discovery.data?.places ?? []}
                    servers={servers}
                    using={checking}
                    onUse={(folder) => {
                        onPath(folder);
                        onCheck(folder, true);
                    }}
                />
            ) : (
                <div className="flex max-w-250 flex-col gap-7 max-md:gap-5">
                    {discovery.isPending && (
                        <div className="grid gap-3 md:grid-cols-2">
                            {[0, 1].map((index) => (
                                <div key={index} className="card flex flex-col gap-3 p-4.5">
                                    <span className="skeleton w-1/3" />
                                    <span className="skeleton w-3/4" />
                                    <span className="skeleton w-1/2" />
                                </div>
                            ))}
                        </div>
                    )}
                    {discovery.isError && (
                        <Notice tone="danger">{words.searchFailed(errorMessage(t, discovery.error))}</Notice>
                    )}
                    {servers.length > 0 && (
                        <section className="flex flex-col gap-3">
                            <h2 className="eyebrow">{words.foundHere}</h2>
                            <ul aria-label={words.servers} className="grid gap-3 md:grid-cols-2">
                                {servers.map((server) => (
                                    <li key={server.path} className="flex">
                                        <ServerCard
                                            server={server}
                                            selected={server === picked}
                                            onPick={() => onPath(server.path)}
                                        />
                                    </li>
                                ))}
                            </ul>
                        </section>
                    )}
                    {picked?.portHolder != null && picked.port !== null && (
                        <Notice tone="warning" live={false}>
                            {words.portTaken(picked.port, picked.portHolder, picked.txAdmin)}
                        </Notice>
                    )}
                    <section className="flex flex-col gap-3">
                        {servers.length > 0 && <h2 className="eyebrow">{words.elsewhere}</h2>}
                        <FolderField
                            value={draft.dataPath}
                            error={picked === null ? refusal : undefined}
                            onChange={onPath}
                            onChoose={(folder) => onCheck(folder, false)}
                            beside={
                                <Button icon={FolderOpen} onClick={() => setBrowsing(true)}>
                                    {words.browse}
                                </Button>
                            }
                        />
                    </section>
                    <div aria-live="polite" className="empty:hidden">
                        {path !== '' && (verdict || (checking && report === null)) && (
                            <div className="card flex flex-col gap-4 p-5 max-md:p-4">
                                {report === null ? (
                                    <span className="flex items-center gap-2 text-small text-ink-3">
                                        <LoaderCircle
                                            size={14}
                                            strokeWidth={1.5}
                                            className="animate-spin"
                                            aria-hidden="true"
                                        />
                                        {words.browser.checking}
                                    </span>
                                ) : (
                                    <FolderVerdict
                                        report={report}
                                        problemShown={picked === null}
                                        suggestionLabel={words.useSuggestion}
                                        onSuggestion={(folder) => {
                                            onPath(folder);
                                            onCheck(folder, false);
                                        }}
                                    />
                                )}
                            </div>
                        )}
                    </div>
                </div>
            )}
        </StepPage>
    );
}

/** A server found on the host, with what Observer read of it. */
function ServerCard({
    server,
    selected,
    onPick,
}: {
    server: FoundServer;
    selected: boolean;
    onPick: () => void;
}) {
    const { t } = useI18n();
    const words = t.serverSetup.folder;
    return (
        <button
            type="button"
            aria-pressed={selected}
            className={cn(
                'card flex w-full flex-col gap-3.5 p-4.5 text-left transition-[border-color,box-shadow] hover:border-line-strong motion-reduce:transition-none',
                selected && 'border-accent ring-3 ring-focus-ring hover:border-accent',
            )}
            onClick={onPick}
        >
            <span className="flex w-full items-center gap-3">
                <span
                    className={cn(
                        'grid size-9 flex-none place-items-center rounded-control',
                        selected ? 'bg-accent-soft text-accent-ink' : 'bg-surface-2 text-ink-2',
                    )}
                >
                    <Server size={18} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <b className="truncate text-card text-ink">{folderName(server.path)}</b>
                    <span className="font-mono text-meta text-ink-3 [overflow-wrap:anywhere]">
                        {server.path}
                    </span>
                </span>
                {selected && (
                    <span className="inline-flex h-6 flex-none items-center gap-1.5 self-start rounded-pill bg-accent px-2.5 text-meta font-semibold text-on-accent">
                        <Check size={12} strokeWidth={3} aria-hidden="true" />
                        {words.selected}
                    </span>
                )}
            </span>
            <span className="flex flex-wrap gap-1.5">
                <Chip>{words.resourceCount(server.resources, server.resourcesTruncated)}</Chip>
                {server.cfg !== null && <Chip className="font-mono">{server.cfg}</Chip>}
                {server.port !== null &&
                    (server.portHolder === null ? (
                        <Chip>{words.port(server.port)}</Chip>
                    ) : (
                        <Chip tone="warning" icon={TriangleAlert}>
                            {words.portInUse(server.port, server.portHolder.program)}
                        </Chip>
                    ))}
                {server.onesync !== null && <Chip>{words.onesync[server.onesync]}</Chip>}
                {server.txAdmin && <Chip tone="accent">{words.usedByTxAdmin}</Chip>}
            </span>
        </button>
    );
}
