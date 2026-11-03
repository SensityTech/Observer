import { useQuery } from '@tanstack/react-query';
import {
    ChevronRight,
    ChevronUp,
    CircleCheck,
    CircleX,
    Folder,
    HardDrive,
    House,
    LoaderCircle,
    Lock,
    Search,
    TriangleAlert,
} from 'lucide-react';
import { useId, useState } from 'react';

import type { FolderEntry } from '../api/generated/FolderEntry';
import type { FolderReport } from '../api/generated/FolderReport';
import type { FoundServer } from '../api/generated/FoundServer';
import type { Place } from '../api/generated/Place';
import { folderCheckQuery, folderQuery } from '../api/serverSetup';
import { Button, buttonVariants } from '../components/Button';
import { Chip } from '../components/Chip';
import { inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { crumbs, folderName, intoFolder } from './paths';

/** Where a path is listed and checked from: without its last separator, but for a root's. */
const settled = (path: string) => crumbs(path).at(-1)?.path ?? path;

/**
 * The host's folders, one at a time: the places to start from on the left, the folders of the
 * one open in the middle, and what Observer finds in the one selected on the right. The folder
 * open is selected until another is.
 */
export function FolderBrowser({
    start,
    chosen,
    places,
    servers,
    using,
    onUse,
}: {
    start: string;
    /** The folder the setup has now, selected at first. */
    chosen: string | null;
    places: Place[];
    servers: FoundServer[];
    /** The folder taken is being checked again before the setup goes on. */
    using: boolean;
    onUse: (path: string) => void;
}) {
    const { t } = useI18n();
    const words = t.serverSetup.folder.browser;
    const filterId = useId();
    const [folder, setFolder] = useState(() => settled(start));
    const [selected, setSelected] = useState(chosen);
    const [filter, setFilter] = useState('');
    const listing = useQuery(folderQuery({ path: intoFolder(folder), prefix: '' }));
    const target = selected ?? folder;

    const go = (path: string, select: string | null = null) => {
        setFolder(settled(path));
        setSelected(select);
        setFilter('');
    };
    const entries = (listing.data?.entries ?? []).filter((entry) =>
        entry.name.toLowerCase().includes(filter.trim().toLowerCase()),
    );
    const parent = listing.data?.parent ?? null;
    const name = folderName(folder);
    const problem = listing.data?.problem ?? null;

    return (
        <section className="card flex flex-wrap overflow-hidden" aria-label={words.label}>
            <div className="flex min-w-0 flex-[999_1_440px] flex-col border-line md:border-r">
                <div className="flex flex-wrap items-center gap-1.5 border-b border-line px-3.5 py-3">
                    <Button
                        size="icon-sm"
                        tone="ghost"
                        aria-label={words.up}
                        disabled={parent === null}
                        onClick={() => parent !== null && go(parent)}
                    >
                        <ChevronUp size={14} strokeWidth={1.5} aria-hidden="true" />
                    </Button>
                    <nav aria-label={words.current} className="min-w-0 flex-[1_1_240px]">
                        <ol className="flex flex-wrap items-center gap-0.5 font-mono text-small">
                            {crumbs(folder).map((crumb, index, all) => {
                                const last = index === all.length - 1;
                                return (
                                    <li key={crumb.path} className="inline-flex items-center gap-0.5">
                                        {index > 0 && (
                                            <ChevronRight
                                                size={12}
                                                strokeWidth={1.5}
                                                className="text-ink-3"
                                                aria-hidden="true"
                                            />
                                        )}
                                        <button
                                            type="button"
                                            aria-current={last ? 'location' : undefined}
                                            className={cn(
                                                'rounded-xs px-1 py-0.5 hover:bg-hover hover:text-ink max-md:min-h-9',
                                                last ? 'text-ink' : 'text-ink-3',
                                            )}
                                            onClick={() => go(crumb.path)}
                                        >
                                            {crumb.name}
                                        </button>
                                    </li>
                                );
                            })}
                        </ol>
                    </nav>
                    <div className="relative w-50 max-md:w-full">
                        <Search
                            size={14}
                            strokeWidth={1.5}
                            className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-ink-3"
                            aria-hidden="true"
                        />
                        <input
                            id={filterId}
                            type="search"
                            aria-label={words.filter}
                            placeholder={words.filterPlaceholder}
                            value={filter}
                            spellCheck={false}
                            autoComplete="off"
                            className={cn(inputClass, 'h-8 pl-8 text-ui')}
                            onChange={(event) => setFilter(event.target.value)}
                        />
                    </div>
                </div>
                <div className="flex min-h-130 max-md:min-h-0 max-md:flex-col">
                    <Places places={places} servers={servers} folder={folder} selected={selected} onGo={go} />
                    <div className="max-h-[min(70vh,560px)] min-w-0 flex-1 overflow-y-auto p-2 scrollbar-thin max-md:max-h-[60vh]">
                        {listing.isPending && (
                            <output className="flex items-center gap-2 px-2.5 py-2 text-small text-ink-3">
                                <LoaderCircle
                                    size={14}
                                    strokeWidth={1.5}
                                    className="animate-spin"
                                    aria-hidden="true"
                                />
                                {words.reading}
                            </output>
                        )}
                        {listing.isError && <Notice tone="danger">{errorMessage(t, listing.error)}</Notice>}
                        {problem !== null && (
                            <p className="px-2.5 py-2 text-small text-ink-2">
                                {problem === 'invalidPath' && listing.data?.pathProblem != null
                                    ? t.server.problems[listing.data.pathProblem]
                                    : t.serverSetup.folder.problems[problem]}
                            </p>
                        )}
                        {listing.data !== undefined && problem === null && (
                            <>
                                <ul aria-label={words.folders(name)} className="flex flex-col gap-0.5">
                                    {entries.map((entry) => (
                                        <Row
                                            key={entry.path}
                                            entry={entry}
                                            selected={entry.path === selected}
                                            onSelect={() => setSelected(entry.path)}
                                            onOpen={() => go(entry.path)}
                                        />
                                    ))}
                                </ul>
                                {entries.length === 0 && (
                                    <p className="px-2.5 py-2 text-small text-ink-3">
                                        {filter.trim() === '' ? words.empty : words.noMatch}
                                    </p>
                                )}
                                {listing.data.truncated && (
                                    <p className="px-2.5 py-2 text-small text-ink-3">{words.truncated}</p>
                                )}
                            </>
                        )}
                    </div>
                </div>
            </div>
            <Preview path={target} using={using} onUse={onUse} onGo={go} />
        </section>
    );
}

/** A place to start from, the one open marked. */
const placeClass = (on: boolean) =>
    cn(
        'flex h-7.5 w-full items-center gap-2.5 rounded-sm px-2.5 text-ui whitespace-nowrap max-md:h-11',
        on ? 'bg-hover text-ink' : 'text-ink-2 hover:bg-hover hover:text-ink',
    );

/** The drives and the home folder, then the servers the search found. */
function Places({
    places,
    servers,
    folder,
    selected,
    onGo,
}: {
    places: Place[];
    servers: FoundServer[];
    folder: string;
    selected: string | null;
    onGo: (path: string, select?: string | null) => void;
}) {
    const { t } = useI18n();
    const words = t.serverSetup.folder.browser;
    return (
        <div className="flex flex-none flex-col gap-0.5 border-line px-2 py-2.5 md:w-45 md:border-r max-md:flex-row max-md:flex-wrap max-md:border-b">
            <p className="eyebrow px-2 py-1 max-md:w-full">{words.places}</p>
            <ul className="contents">
                {places.map((place) => {
                    const Icon = place.kind === 'home' ? House : HardDrive;
                    return (
                        <li key={place.path} className="max-md:w-auto">
                            <button
                                type="button"
                                className={placeClass(settled(place.path) === folder)}
                                aria-current={settled(place.path) === folder ? 'location' : undefined}
                                onClick={() => onGo(place.path)}
                            >
                                <Icon size={14} strokeWidth={1.5} aria-hidden="true" />
                                {place.kind === 'home' ? words.home : folderName(place.path)}
                            </button>
                        </li>
                    );
                })}
            </ul>
            {servers.length > 0 && (
                <>
                    <p className="eyebrow px-2 pt-3 pb-1 max-md:w-full max-md:pt-1">{words.found}</p>
                    <ul className="contents">
                        {servers.map((server) => {
                            const into = crumbs(server.path).at(-2)?.path ?? server.path;
                            return (
                                <li key={server.path}>
                                    <button
                                        type="button"
                                        className={placeClass(selected === server.path)}
                                        title={server.path}
                                        onClick={() => onGo(into, server.path)}
                                    >
                                        <span className="led led-ok" aria-hidden="true" />
                                        {folderName(server.path)}
                                    </button>
                                </li>
                            );
                        })}
                    </ul>
                </>
            )}
        </div>
    );
}

/** A folder of the one open: selected by a press, gone into by its arrow or a double press. */
function Row({
    entry,
    selected,
    onSelect,
    onOpen,
}: {
    entry: FolderEntry;
    selected: boolean;
    onSelect: () => void;
    onOpen: () => void;
}) {
    const { t } = useI18n();
    const words = t.serverSetup.folder;
    const readable = entry.kind !== 'unreadable';
    const Icon = readable ? Folder : Lock;
    return (
        <li
            className={cn(
                'flex items-center gap-1 rounded-sm pr-1',
                selected && 'bg-accent-soft shadow-[inset_2px_0_0_var(--accent)]',
            )}
        >
            <button
                type="button"
                aria-pressed={selected}
                disabled={!readable}
                className={cn(
                    'flex h-10 min-w-0 flex-1 items-center gap-2.5 rounded-sm px-2.5 text-left max-md:h-11',
                    readable ? 'hover:bg-hover' : 'cursor-not-allowed text-ink-3',
                )}
                onClick={onSelect}
                onDoubleClick={onOpen}
            >
                <Icon
                    size={14}
                    strokeWidth={1.5}
                    className={cn('flex-none', selected ? 'text-ink' : 'text-ink-3')}
                    aria-hidden="true"
                />
                <span
                    className={cn(
                        'min-w-0 truncate text-body',
                        selected ? 'font-medium text-ink' : readable && 'text-ink-2',
                    )}
                >
                    {entry.name}
                </span>
                <span className="ml-auto flex flex-none items-center gap-2">
                    {entry.link && <span className="text-small text-ink-3">{words.link}</span>}
                    {entry.kind === 'server' && <Chip tone="success">{words.kinds.server}</Chip>}
                    {entry.kind === 'txAdminData' && <Chip>{words.kinds.txAdminData}</Chip>}
                    {entry.kind === 'folder' && entry.inside.some((inside) => inside.kind === 'server') && (
                        <span className="text-small text-ink-3 max-md:hidden">
                            {words.browser.holdsServer}
                        </span>
                    )}
                    {!readable && <span className="text-small max-md:hidden">{words.kinds.unreadable}</span>}
                </span>
            </button>
            {readable && (
                <button
                    type="button"
                    aria-label={words.browser.open(entry.name)}
                    className={cn(buttonVariants({ size: 'icon-sm', tone: 'ghost' }))}
                    onClick={onOpen}
                >
                    <ChevronRight size={14} strokeWidth={1.5} aria-hidden="true" />
                </button>
            )}
        </li>
    );
}

/** What the folder check finds in the folder selected, and the button that takes it. */
function Preview({
    path,
    using,
    onUse,
    onGo,
}: {
    path: string;
    using: boolean;
    onUse: (path: string) => void;
    onGo: (path: string, select?: string | null) => void;
}) {
    const { t } = useI18n();
    const words = t.serverSetup.folder;
    const check = useQuery(folderCheckQuery(path));
    const report = check.data;
    return (
        <aside
            aria-label={words.browser.selection}
            className="flex min-w-0 flex-[1_1_300px] flex-col gap-4 bg-bg-subtle p-5 max-md:p-4"
        >
            <div className="flex flex-col gap-1">
                <span className="eyebrow">{words.browser.selected}</span>
                <span className="font-mono text-small text-ink [overflow-wrap:anywhere]">{path}</span>
            </div>
            <div aria-live="polite" className="flex flex-col gap-4">
                {check.isPending && (
                    <p className="flex items-center gap-2 text-small text-ink-3">
                        <LoaderCircle
                            size={14}
                            strokeWidth={1.5}
                            className="animate-spin"
                            aria-hidden="true"
                        />
                        {words.browser.checking}
                    </p>
                )}
                {check.isError && <Notice tone="danger">{errorMessage(t, check.error)}</Notice>}
                {report !== undefined && <Verdict report={report} onGo={onGo} />}
            </div>
            <Button
                tone="ink"
                className="mt-auto w-full"
                disabled={report?.problem !== null}
                pending={using}
                onClick={() => onUse(path)}
            >
                {words.browser.use}
            </Button>
        </aside>
    );
}

function Verdict({
    report,
    onGo,
}: {
    report: FolderReport;
    onGo: (path: string, select?: string | null) => void;
}) {
    const { t, format } = useI18n();
    const words = t.serverSetup.folder;
    const resources = report.resources;
    if (report.problem !== null || resources === null) {
        const problem = report.problem ?? 'noResources';
        const suggestion = report.suggestion;
        return (
            <>
                <p className="flex items-start gap-2 text-body text-ink-2">
                    <CircleX
                        size={16}
                        strokeWidth={1.5}
                        className="mt-0.5 flex-none text-ink-3"
                        aria-hidden="true"
                    />
                    {problem === 'invalidPath' && report.pathProblem !== null
                        ? t.server.problems[report.pathProblem]
                        : words.problems[problem]}
                </p>
                {suggestion !== null && (
                    <div className="flex flex-col items-start gap-2">
                        <p className="text-small text-ink-2">
                            {words.suggestion}{' '}
                            <span className="font-mono text-ink [overflow-wrap:anywhere]">{suggestion}</span>
                        </p>
                        <Button
                            size="sm"
                            onClick={() => onGo(crumbs(suggestion).at(-2)?.path ?? suggestion, suggestion)}
                        >
                            {words.browser.showSuggestion}
                        </Button>
                    </div>
                )}
            </>
        );
    }
    const detected = report.detectedCfg;
    return (
        <>
            <p className="flex items-center gap-2 text-body font-medium text-success">
                <CircleCheck size={16} strokeWidth={1.5} className="flex-none" aria-hidden="true" />
                {words.browser.isServer}
            </p>
            <dl className="grid grid-cols-2 gap-3">
                <div className="rounded-box border border-line bg-surface p-3">
                    <dt className="eyebrow">{words.browser.resources}</dt>
                    <dd className="text-figure-sm text-ink tabular-nums">
                        {format.number(resources.resources)}
                    </dd>
                    <dd className="text-small text-ink-3">
                        {words.browser.inCategories(resources.categories)}
                    </dd>
                </div>
                <div className="rounded-box border border-line bg-surface p-3">
                    <dt className="eyebrow">{words.browser.cfgCount}</dt>
                    <dd className="text-figure-sm text-ink tabular-nums">
                        {format.number(report.cfgFiles.length)}
                    </dd>
                    {detected !== null && (
                        <dd className="text-small text-ink-3 [overflow-wrap:anywhere]">
                            {words.browser.cfgFirst(detected)}
                        </dd>
                    )}
                </div>
            </dl>
            {report.cfgFiles.length > 0 && (
                <ul className="flex flex-wrap gap-1.5" aria-label={words.cfgFiles}>
                    {report.cfgFiles.map((file) => (
                        <li
                            key={file}
                            className="rounded-xs border border-line bg-surface-2 px-1.5 py-0.5 font-mono text-small text-ink-2 [overflow-wrap:anywhere]"
                        >
                            {file}
                        </li>
                    ))}
                </ul>
            )}
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
        </>
    );
}
