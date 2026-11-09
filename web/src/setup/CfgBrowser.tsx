import { useQuery } from '@tanstack/react-query';
import {
    ChevronDown,
    ChevronRight,
    EyeOff,
    File,
    FileCog,
    Folder,
    FolderOpen,
    Info,
    Link2,
    Lock,
    OctagonX,
    TriangleAlert,
} from 'lucide-react';
import { useEffect, useRef } from 'react';

import type { CfgFinding } from '../api/generated/CfgFinding';
import type { FileEntry } from '../api/generated/FileEntry';
import type { Severity } from '../api/generated/Severity';
import { cfgTextQuery, filesQuery } from '../api/serverSetup';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { keyedTokens, type TokenKind } from './cfgSyntax';

/** The folders that hold the cfg files a folder check found, the root among them, for the tree to open on. */
export const foldersOf = (files: readonly string[]) => {
    const folders = new Set<string>(['']);
    for (const file of files) {
        const parts = file.split('/').slice(0, -1);
        parts.forEach((_, index) => folders.add(parts.slice(0, index + 1).join('/')));
    }
    return folders;
};

type TreeProps = {
    dataPath: string;
    /** The cfg FXServer runs. */
    chosen: string;
    /** The file the preview shows. */
    previewed: string;
    expanded: ReadonlySet<string>;
    onlyCfg: boolean;
    filter: string;
    onToggle: (folder: string) => void;
    onOpen: (cfg: string) => void;
};

/** The server folder as a tree, each folder read when it opens; only its cfg files can be opened. */
export function FileTree(props: TreeProps) {
    const { t } = useI18n();
    return (
        <ul aria-label={t.serverSetup.cfg.browser.tree} className="flex flex-col gap-px">
            <FolderEntries {...props} path="" depth={0} />
        </ul>
    );
}

function FolderEntries({ path, depth, ...props }: TreeProps & { path: string; depth: number }) {
    const { t } = useI18n();
    const words = t.serverSetup.cfg.browser;
    const listing = useQuery(filesQuery({ dataPath: props.dataPath, path }));
    const indent = { paddingInlineStart: `${8 + depth * 16}px` };
    if (listing.isPending) {
        return (
            <li style={indent} className="py-1.5" aria-busy="true">
                <span className="sr-only">{t.common.loading}</span>
                <span className="skeleton block h-4 w-3/4" />
            </li>
        );
    }
    if (listing.isError) {
        return (
            <li style={indent} className="py-1.5 text-small text-danger-ink">
                {errorMessage(t, listing.error)}
            </li>
        );
    }
    if (listing.data.problem !== null) {
        return (
            <li style={indent} className="py-1.5 text-small text-ink-3">
                {words.cannotOpen}
            </li>
        );
    }
    const needle = props.filter.trim().toLowerCase();
    const entries = listing.data.entries.filter(
        (entry) =>
            entry.kind === 'folder' ||
            entry.kind === 'withheld' ||
            ((entry.kind === 'cfg' || !props.onlyCfg) && entry.name.toLowerCase().includes(needle)),
    );
    if (entries.length === 0) {
        return (
            <li style={indent} className="py-1.5 text-small text-ink-3">
                {needle === '' ? words.empty : words.noMatch}
            </li>
        );
    }
    return (
        <>
            {entries.map((entry) => (
                <EntryRow key={entry.path} entry={entry} depth={depth} {...props} />
            ))}
            {listing.data.truncated && (
                <li style={indent} className="py-1.5 text-small text-ink-3">
                    {words.truncated}
                </li>
            )}
        </>
    );
}

const ROW =
    'flex h-8 w-full min-w-0 items-center gap-1.5 rounded-sm pr-2 text-left text-ui max-md:h-11 focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent';

function EntryRow({ entry, depth, ...props }: TreeProps & { entry: FileEntry; depth: number }) {
    const { t } = useI18n();
    const words = t.serverSetup.cfg.browser;
    const indent = { paddingInlineStart: `${8 + depth * 16}px` };
    const spacer = <span className="w-3 flex-none" aria-hidden="true" />;

    if (entry.kind === 'folder' && !entry.link) {
        const open = props.expanded.has(entry.path);
        const Chevron = open ? ChevronDown : ChevronRight;
        const Icon = open ? FolderOpen : Folder;
        return (
            <li>
                <button
                    type="button"
                    aria-expanded={open}
                    style={indent}
                    className={cn(ROW, 'hover:bg-hover')}
                    onClick={() => props.onToggle(entry.path)}
                >
                    <Chevron size={12} strokeWidth={2} className="flex-none text-ink-3" aria-hidden="true" />
                    <Icon
                        size={15}
                        strokeWidth={1.75}
                        className={cn('flex-none', open ? 'text-ink-2' : 'text-ink-3')}
                        aria-hidden="true"
                    />
                    <span className="truncate text-ink">{entry.name}</span>
                </button>
                {open && (
                    <ul className="flex flex-col gap-px">
                        <FolderEntries {...props} path={entry.path} depth={depth + 1} />
                    </ul>
                )}
            </li>
        );
    }

    if (entry.kind === 'cfg') {
        const previewed = entry.path === props.previewed;
        const chosen = entry.path === props.chosen;
        return (
            <li>
                <button
                    type="button"
                    aria-current={previewed ? 'true' : undefined}
                    style={indent}
                    className={cn(ROW, previewed ? 'bg-accent-soft' : 'hover:bg-hover')}
                    onClick={() => props.onOpen(entry.path)}
                >
                    {spacer}
                    <FileCog
                        size={15}
                        strokeWidth={1.75}
                        className="flex-none text-accent-ink"
                        aria-hidden="true"
                    />
                    <span
                        className={cn('truncate font-mono text-small text-ink', previewed && 'font-semibold')}
                    >
                        {entry.name}
                    </span>
                    {entry.link && (
                        <Link2
                            size={12}
                            strokeWidth={2}
                            className="flex-none text-ink-3"
                            aria-label={words.link}
                        />
                    )}
                    {chosen && (
                        <span className="ml-auto flex-none rounded-pill bg-success-soft px-1.75 py-px text-tag font-semibold text-success">
                            {words.runs}
                        </span>
                    )}
                </button>
            </li>
        );
    }

    const withheld = entry.kind === 'withheld';
    const Icon = withheld ? Lock : entry.kind === 'folder' ? Folder : File;
    return (
        <li
            style={indent}
            className={cn(ROW, 'cursor-default text-ink-3')}
            title={withheld ? words.withheld : entry.link ? words.link : undefined}
        >
            {spacer}
            <Icon size={15} strokeWidth={1.75} className="flex-none" aria-hidden="true" />
            <span className="truncate">{entry.name}</span>
            {entry.link && <Link2 size={12} strokeWidth={2} className="flex-none" aria-hidden="true" />}
            {withheld && (
                <span className="ml-auto flex-none text-micro whitespace-nowrap">{words.neverRead}</span>
            )}
        </li>
    );
}

const TOKEN_CLASS = {
    command: 'text-accent-ink',
    string: 'text-[color:var(--term-green)]',
    number: 'text-warning',
    comment: 'text-ink-3',
    plain: 'text-[color:var(--term-fg)]',
    secret: '',
} satisfies Record<TokenKind, string>;

const MARK = {
    blocking: { icon: OctagonX, ink: 'text-danger-ink', row: 'bg-danger-soft' },
    warning: { icon: TriangleAlert, ink: 'text-warning', row: 'bg-warning-soft' },
    note: { icon: Info, ink: 'text-accent-ink', row: 'bg-accent-soft' },
} as const satisfies Record<Severity, unknown>;

const RANK = { blocking: 0, warning: 1, note: 2 } satisfies Record<Severity, number>;

/**
 * A cfg's text as Observer reads it, its secrets left out: the lines numbered and coloured, those
 * a finding names marked with its severity. `focus` scrolls a line into view.
 */
export function CfgPreview({
    dataPath,
    path,
    findings,
    focus,
}: {
    dataPath: string;
    path: string;
    findings: readonly CfgFinding[];
    focus: number | null;
}) {
    const { t, format } = useI18n();
    const words = t.serverSetup.cfg.browser;
    const text = useQuery(cfgTextQuery({ dataPath, cfgFile: path }));
    const body = useRef<HTMLOListElement>(null);
    const marks = new Map<number, Severity>();
    for (const finding of findings) {
        if (finding.file === path && finding.line !== null) {
            const held = marks.get(finding.line);
            if (held === undefined || RANK[finding.severity] < RANK[held]) {
                marks.set(finding.line, finding.severity);
            }
        }
    }
    const lines = text.data?.lines ?? [];
    const hidden = lines.some((line) => line.some((span) => span.secret));

    useEffect(() => {
        if (focus !== null && lines.length > 0) {
            body.current?.querySelector(`[data-line="${focus}"]`)?.scrollIntoView({ block: 'center' });
        }
    }, [focus, lines.length]);

    return (
        <div className="flex min-h-0 flex-1 flex-col">
            <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5 border-b border-line px-4 py-3">
                <span className="min-w-0 font-mono text-ui font-medium text-ink [overflow-wrap:anywhere]">
                    {path}
                </span>
                {text.data?.problem === null && (
                    <span className="text-meta text-ink-3">{words.lines(lines.length)}</span>
                )}
                {hidden && (
                    <span className="ml-auto inline-flex items-center gap-1.5 text-meta text-ink-3">
                        <EyeOff size={13} strokeWidth={1.75} aria-hidden="true" />
                        {words.hidden}
                    </span>
                )}
            </div>
            {text.isPending && (
                <div className="flex flex-col gap-2 p-4" aria-busy="true">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-2/3" />
                    <span className="skeleton w-1/2" />
                    <span className="skeleton w-3/5" />
                </div>
            )}
            {text.isError && <p className="p-4 text-small text-danger-ink">{errorMessage(t, text.error)}</p>}
            {text.data !== undefined && (text.data.folder !== null || text.data.problem !== null) && (
                <p className="p-4 text-small text-ink-3">
                    {text.data.problem !== null
                        ? t.serverSetup.cfg.problems[text.data.problem]
                        : words.cannotOpen}
                </p>
            )}
            {text.data?.problem === null && text.data.folder === null && (
                <ol
                    ref={body}
                    aria-label={words.text(path)}
                    className="max-h-140 min-h-0 flex-1 overflow-auto bg-bg py-2.5 font-mono text-small leading-[22px] max-md:max-h-100"
                >
                    {lines.map((spans, index) => {
                        const number = index + 1;
                        const mark = marks.get(number);
                        const Mark = mark === undefined ? null : MARK[mark].icon;
                        return (
                            <li
                                key={number}
                                data-line={number}
                                className={cn(
                                    'flex min-h-[22px] items-start',
                                    mark !== undefined && MARK[mark].row,
                                )}
                            >
                                <span
                                    className="w-11 flex-none pr-2.5 text-right text-ink-3 select-none"
                                    aria-hidden="true"
                                >
                                    {format.number(number, { useGrouping: false })}
                                </span>
                                <span className="flex h-[22px] w-4 flex-none items-center">
                                    {Mark !== null && mark !== undefined && (
                                        <Mark
                                            size={12}
                                            strokeWidth={2.25}
                                            className={MARK[mark].ink}
                                            aria-label={t.serverSetup.cfg.severities[mark]}
                                        />
                                    )}
                                </span>
                                <span className="pr-4 whitespace-pre-wrap [overflow-wrap:anywhere]">
                                    {keyedTokens(spans).map(({ token, key }) =>
                                        token.kind === 'secret' ? (
                                            <span
                                                key={key}
                                                className="mx-px rounded-xs bg-surface-3 px-1.5 text-ink-3"
                                                title={words.secret}
                                            >
                                                ••••••••<span className="sr-only">{words.secret}</span>
                                            </span>
                                        ) : (
                                            <span key={key} className={TOKEN_CLASS[token.kind]}>
                                                {token.text}
                                            </span>
                                        ),
                                    )}
                                </span>
                            </li>
                        );
                    })}
                </ol>
            )}
            {text.data?.truncated === true && (
                <p className="border-t border-line px-4 py-2 text-meta text-ink-3">{words.longFile}</p>
            )}
        </div>
    );
}
