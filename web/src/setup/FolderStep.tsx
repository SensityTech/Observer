import { useQuery } from '@tanstack/react-query';
import {
    ArrowLeft,
    Check,
    ChevronDown,
    CircleCheck,
    FolderOpen,
    FolderSearch,
    RotateCw,
    Server,
    TriangleAlert,
} from 'lucide-react';
import { useEffect, useId, useRef, useState, type Ref } from 'react';

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
import { crumbs, folderName } from './paths';
import type { Checked } from './state';
import { StepActions } from './StepActions';

/**
 * The server folder: one of those Observer found on the host, a path typed with the host's folders
 * offered as it goes, or a folder picked in the browser. Continue checks it first when it was not.
 */
export function FolderStep({
    titleRef,
    draft,
    checked,
    checking,
    onPath,
    onCheck,
    onContinue,
}: {
    titleRef: Ref<HTMLHeadingElement>;
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
    const headingId = useId();
    const discovery = useQuery(discoveryQuery);
    const servers = discovery.data?.servers ?? [];
    const path = draft.dataPath.trim();
    const picked = servers.find((server) => server.path === path) ?? null;
    const [browsing, setBrowsing] = useState(false);
    const [elsewhere, setElsewhere] = useState<boolean | null>(null);
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

    // Open until the owner closes it: when nothing was found, or the folder is none of those found.
    const open =
        elsewhere ??
        (discovery.isError ||
            (discovery.isSuccess && (servers.length === 0 || (path !== '' && picked === null))));
    const problem = report?.problem ?? null;
    const refusal =
        problem === null
            ? undefined
            : problem === 'invalidPath' && report?.pathProblem != null
              ? t.server.problems[report.pathProblem]
              : words.problems[problem];
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
        <section aria-labelledby={headingId} className="flex flex-col gap-4.5">
            <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="flex min-w-0 flex-[1_1_320px] flex-col gap-1.5">
                    <h2
                        id={headingId}
                        ref={titleRef}
                        tabIndex={-1}
                        className="text-state text-ink outline-none"
                    >
                        {words.question}
                    </h2>
                    <output className="text-body text-ink-2">{lead}</output>
                </div>
                {browsing && (
                    <Button
                        size="sm"
                        tone="ghost"
                        icon={ArrowLeft}
                        onClick={() => {
                            setBrowsing(false);
                            setElsewhere(true);
                        }}
                    >
                        {words.browser.typeInstead}
                    </Button>
                )}
            </div>

            {browsing ? (
                <FolderBrowser
                    start={start}
                    chosen={path === '' ? null : path}
                    places={discovery.data?.places ?? []}
                    servers={servers}
                    using={checking}
                    onUse={(folder) => {
                        onPath(folder);
                        onCheck(folder, true);
                    }}
                />
            ) : (
                <>
                    {discovery.isPending && (
                        <div className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,18rem),1fr))] gap-3">
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
                        <ul
                            aria-label={words.servers}
                            className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,18rem),1fr))] gap-3"
                        >
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
                    )}
                    {picked?.portHolder != null && picked.port !== null && (
                        <Notice tone="warning" live={false}>
                            {words.portTaken(picked.port, picked.portHolder, picked.txAdmin)}
                        </Notice>
                    )}
                    {picked !== null && refusal !== undefined && <Notice tone="danger">{refusal}</Notice>}

                    <details
                        className="card group"
                        open={open}
                        onToggle={(event) => setElsewhere(event.currentTarget.open)}
                    >
                        <summary className="flex cursor-pointer list-none items-center gap-2.5 rounded-card px-4.5 py-3.5 text-body font-medium text-ink max-md:min-h-11 [&::-webkit-details-marker]:hidden">
                            <FolderSearch
                                size={14}
                                strokeWidth={1.5}
                                className="text-ink-3"
                                aria-hidden="true"
                            />
                            {words.elsewhere}
                            <ChevronDown
                                size={14}
                                strokeWidth={1.5}
                                className="ml-auto text-ink-3 transition-transform group-open:rotate-180 motion-reduce:transition-none"
                                aria-hidden="true"
                            />
                        </summary>
                        <div className="flex flex-col gap-4 px-4.5 pb-4.5">
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
                            {picked === null && report !== null && (
                                <Outcome
                                    report={report}
                                    onUse={(folder) => {
                                        onPath(folder);
                                        onCheck(folder, false);
                                    }}
                                />
                            )}
                        </div>
                    </details>
                </>
            )}

            {/* The browser's own button takes the folder it shows. */}
            {!browsing && (
                <StepActions note={words.lead}>
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
            )}
        </section>
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
                'card flex w-full flex-col gap-3 p-4.5 text-left transition-[border-color,box-shadow] hover:border-line-strong motion-reduce:transition-none',
                selected && 'border-accent ring-3 ring-focus-ring hover:border-accent',
            )}
            onClick={onPick}
        >
            <span className="flex w-full items-center gap-2.5">
                <span className="grid size-7 flex-none place-items-center rounded-sm bg-surface-2 text-ink-2">
                    <Server size={14} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <b className="min-w-0 truncate text-card text-ink">{folderName(server.path)}</b>
                {selected && (
                    <span className="ml-auto inline-flex flex-none items-center gap-1.5 text-small text-accent-ink">
                        <Check size={14} strokeWidth={1.75} aria-hidden="true" />
                        {words.selected}
                    </span>
                )}
            </span>
            <span className="font-mono text-meta text-ink-3 [overflow-wrap:anywhere]">{server.path}</span>
            <span className="flex flex-wrap gap-1.5">
                <Chip>{words.resourceCount(server.resources, server.resourcesTruncated)}</Chip>
                {server.cfg !== null && <Chip className="font-mono">{server.cfg}</Chip>}
                {server.onesync !== null && <Chip>{words.onesync[server.onesync]}</Chip>}
                {server.txAdmin && <Chip>{words.usedByTxAdmin}</Chip>}
                {server.portHolder !== null && server.port !== null && (
                    <Chip tone="warning" icon={TriangleAlert}>
                        {words.portInUse(server.port)}
                    </Chip>
                )}
            </span>
        </button>
    );
}

/** What the folder check said of a path typed: why it is refused and a folder nearby, or what it holds. */
function Outcome({ report, onUse }: { report: FolderReport; onUse: (path: string) => void }) {
    const { t } = useI18n();
    const words = t.serverSetup.folder;
    const resources = report.resources;
    const suggestion = report.suggestion;
    if (suggestion !== null) {
        return (
            <div className="flex flex-col items-start gap-2">
                <p className="text-small text-ink-2">
                    {words.suggestion}{' '}
                    <span className="font-mono text-ink [overflow-wrap:anywhere]">{suggestion}</span>
                </p>
                <Button
                    size="sm"
                    icon={FolderSearch}
                    className="max-md:w-full"
                    onClick={() => onUse(suggestion)}
                >
                    {words.useSuggestion}
                </Button>
            </div>
        );
    }
    if (report.problem !== null || resources === null) {
        return null;
    }
    return (
        <output className="flex flex-col gap-3">
            <p className="flex items-center gap-2 text-body font-medium text-success">
                <CircleCheck size={16} strokeWidth={1.5} className="flex-none" aria-hidden="true" />
                {words.accepted}
            </p>
            <p className="text-body text-ink-2">
                {words.resources(resources.resources, resources.categories, resources.truncated)}
            </p>
            {resources.withoutManifest.length > 0 && (
                <p className="flex items-start gap-2 text-small text-warning">
                    <TriangleAlert
                        size={14}
                        strokeWidth={1.5}
                        className="mt-0.5 flex-none"
                        aria-hidden="true"
                    />
                    <span className="[overflow-wrap:anywhere]">
                        {words.withoutManifest(resources.withoutManifest)}
                    </span>
                </p>
            )}
            {report.cfgFiles.length === 0 ? (
                <p className="text-small text-ink-3">{words.noCfgFile}</p>
            ) : (
                <div className="flex flex-col gap-1.5">
                    <p className="text-ui font-medium text-ink">{words.cfgFiles}</p>
                    <ul className="flex flex-wrap gap-1.5">
                        {report.cfgFiles.map((file) => (
                            <li
                                key={file}
                                className="rounded-xs border border-line bg-surface-2 px-1.5 py-0.5 font-mono text-small text-ink-2 [overflow-wrap:anywhere]"
                            >
                                {file}
                            </li>
                        ))}
                    </ul>
                </div>
            )}
        </output>
    );
}
