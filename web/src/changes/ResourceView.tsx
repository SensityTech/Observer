import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Eraser, FileStack, History, Lock, Package, TriangleAlert } from 'lucide-react';
import { useState, type ReactNode } from 'react';

import { historyQuery, purgeContents } from '../api/changes';
import type { FileChange } from '../api/generated/FileChange';
import type { GitHead } from '../api/generated/GitHead';
import { Permission } from '../api/generated/Permission';
import type { Purged } from '../api/generated/Purged';
import type { ResourceDetail } from '../api/generated/ResourceDetail';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { Dialog } from '../components/Dialog';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { Toggle } from './controls';
import { DiffView } from './DiffView';
import { FILE_CHIP, RESOURCE_CHIP } from './words';

/** A path inside the resource, from its folder on either side. */
const inside = (detail: ResourceDetail, path: string) => {
    for (const root of [detail.change.path, detail.change.fromPath]) {
        if (root !== null && path.startsWith(`${root}/`)) {
            return path.slice(root.length + 1);
        }
    }
    return path;
};

const folderOf = (relative: string) => {
    const slash = relative.lastIndexOf('/');
    return slash === -1 ? '' : relative.slice(0, slash);
};

/** Its files by folder, unchanged ones on request, and the diff of the one picked beside them. */
export function ResourceView({ detail, from, to }: { detail: ResourceDetail; from: number; to: number }) {
    const { t, format } = useI18n();
    const me = useMe();
    const content = allows(me, Permission.ChangesContent);
    const [unchanged, setUnchanged] = useState(false);
    const [picked, setPicked] = useState<string | null>(null);
    const [purged, setPurged] = useState<{ path: string; result: Purged } | null>(null);
    const shown = detail.files.filter((file) => unchanged || file.change !== 'unchanged');
    const current = shown.find((file) => file.path === picked) ?? (content ? shown[0] : undefined);
    const at = current === undefined ? -1 : shown.indexOf(current);
    const step = (by: number) => {
        const next = shown[at + by];
        return next === undefined ? undefined : () => setPicked(next.path);
    };

    return (
        <>
            <ResourceHead detail={detail} />
            {purged !== null && (
                <Notice tone="success">
                    {t.changes.resource.purged(
                        inside(detail, purged.path),
                        purged.result.versions,
                        format.bytes(purged.result.bytes),
                    )}
                </Notice>
            )}
            <div className="grid grid-cols-1 gap-3 2xl:grid-cols-[320px_minmax(0,1fr)] 2xl:items-start">
                <FilesCard
                    detail={detail}
                    files={shown}
                    current={content ? (current?.path ?? null) : null}
                    unchanged={unchanged}
                    onUnchanged={setUnchanged}
                    onPick={content ? setPicked : undefined}
                    onPurged={(path, result) => setPurged({ path, result })}
                />
                {!content ? (
                    <Notice>{t.changes.resource.noContent}</Notice>
                ) : current === undefined ? (
                    <p className="rounded-card border border-dashed border-line-strong bg-bg-subtle px-6 py-10 text-center text-ui text-ink-3">
                        {shown.length === 0 ? t.changes.resource.noFile : t.changes.resource.pickFile}
                    </p>
                ) : (
                    <DiffView
                        key={`${current.path} ${from} ${to}`}
                        from={from}
                        to={to}
                        path={current.path}
                        file={current}
                        onPrevious={step(-1)}
                        onNext={step(1)}
                    />
                )}
            </div>
            <HistoryCard name={detail.change.name} />
        </>
    );
}

const gitWords = (head: GitHead | null, detached: string) =>
    head === null ? null : `${head.branch ?? detached} @ ${head.commit?.slice(0, 10) ?? '?'}`;

function ResourceHead({ detail }: { detail: ResourceDetail }) {
    const { t } = useI18n();
    const words = t.changes.resource;
    const { change } = detail;
    const chip = RESOURCE_CHIP[change.change];
    const versions = [change.fromVersion, change.toVersion];
    const git = [gitWords(detail.fromGit, words.detached), gitWords(detail.toGit, words.detached)];
    const facts: { key: string; label: string; body: ReactNode }[] = [
        {
            key: 'path',
            label: words.path,
            body: (
                <span className="font-mono text-small text-ink [overflow-wrap:anywhere]">
                    {change.fromPath === null ? change.path : t.changes.fromTo(change.fromPath, change.path)}
                </span>
            ),
        },
        {
            key: 'version',
            label: words.version,
            body: (
                <span className="font-mono text-small text-ink">
                    {versions[0] === versions[1]
                        ? (versions[1] ?? words.noVersion)
                        : t.changes.fromTo(versions[0] ?? words.noVersion, versions[1] ?? words.noVersion)}
                </span>
            ),
        },
    ];
    if (git[0] !== null || git[1] !== null) {
        facts.push({
            key: 'git',
            label: words.git,
            body: (
                <span className="font-mono text-small text-ink [overflow-wrap:anywhere]">
                    {git[0] === git[1] ? git[1] : t.changes.fromTo(git[0] ?? '?', git[1] ?? '?')}
                </span>
            ),
        });
    }
    if (detail.packages.length > 0) {
        facts.push({
            key: 'packages',
            label: words.packages,
            body: (
                <ul className="flex flex-col gap-0.5 font-mono text-small text-ink">
                    {detail.packages.map((item) => (
                        <li key={item.name}>
                            {item.name}{' '}
                            <span className="text-ink-2">
                                {item.from === null
                                    ? words.packageAdded(item.to ?? '?')
                                    : item.to === null
                                      ? words.packageRemoved(item.from)
                                      : t.changes.fromTo(item.from, item.to)}
                            </span>
                        </li>
                    ))}
                </ul>
            ),
        });
    }
    return (
        <section className="card flex flex-col gap-4 p-5 max-md:p-4" aria-label={change.name}>
            <div className="flex flex-wrap items-center gap-2">
                <Chip tone={chip.tone} icon={chip.icon}>
                    {t.changes.resources.kinds[change.change]}
                </Chip>
                {change.escrowed && <Chip icon={Lock}>{t.changes.resources.escrowed}</Chip>}
                {change.lines !== null && (
                    <span className="font-mono text-small text-ink-2">
                        {t.changes.lines(String(change.lines.added), String(change.lines.removed))}
                    </span>
                )}
            </div>
            {detail.duplicates.length > 0 && (
                <Notice tone="warning">{words.duplicate(detail.duplicates)}</Notice>
            )}
            <dl className="grid grid-cols-1 gap-x-6 gap-y-3 text-ui md:grid-cols-[minmax(0,10rem)_minmax(0,1fr)]">
                {facts.map((fact) => (
                    <div key={fact.key} className="contents">
                        <dt className="text-ink-3 max-md:-mb-2">{fact.label}</dt>
                        <dd className="min-w-0">{fact.body}</dd>
                    </div>
                ))}
            </dl>
        </section>
    );
}

function FilesCard({
    detail,
    files,
    current,
    unchanged,
    onUnchanged,
    onPick,
    onPurged,
}: {
    detail: ResourceDetail;
    files: FileChange[];
    current: string | null;
    unchanged: boolean;
    onUnchanged: (shown: boolean) => void;
    /** None for an account that cannot read contents: the list is all it gets. */
    onPick: ((path: string) => void) | undefined;
    onPurged: (path: string, purged: Purged) => void;
}) {
    const { t } = useI18n();
    const words = t.changes.resource;
    const me = useMe();
    const [purging, setPurging] = useState<FileChange | null>(null);
    const folders = new Map<string, FileChange[]>();
    for (const file of files) {
        const folder = folderOf(inside(detail, file.path));
        folders.set(folder, [...(folders.get(folder) ?? []), file]);
    }
    return (
        <Card
            id="changes-files"
            icon={FileStack}
            title={words.files}
            aside={
                <Toggle pressed={unchanged} onChange={onUnchanged}>
                    {words.showUnchanged}
                </Toggle>
            }
            flush
        >
            {files.length === 0 ? (
                <p className="px-6 py-6 text-center text-ui text-ink-3">{words.noFile}</p>
            ) : (
                <div className="flex flex-col py-2">
                    {[...folders].map(([folder, inFolder]) => (
                        <div key={folder}>
                            <h3 className="px-5 pt-2 pb-1 font-mono text-meta text-ink-3 [overflow-wrap:anywhere] max-md:px-4">
                                {folder === '' ? words.root : `${folder}/`}
                            </h3>
                            <ul className="flex flex-col">
                                {inFolder.map((file) => (
                                    <FileRow
                                        key={file.path}
                                        detail={detail}
                                        file={file}
                                        current={file.path === current}
                                        onPick={onPick}
                                        onPurge={
                                            me.isOwner && file.kind === 'text'
                                                ? () => setPurging(file)
                                                : undefined
                                        }
                                    />
                                ))}
                            </ul>
                        </div>
                    ))}
                </div>
            )}
            {purging !== null && (
                <Purge
                    file={purging}
                    onClose={() => setPurging(null)}
                    onPurged={(result) => {
                        setPurging(null);
                        onPurged(purging.path, result);
                    }}
                />
            )}
        </Card>
    );
}

function FileRow({
    detail,
    file,
    current,
    onPick,
    onPurge,
}: {
    detail: ResourceDetail;
    file: FileChange;
    current: boolean;
    onPick: ((path: string) => void) | undefined;
    onPurge: (() => void) | undefined;
}) {
    const { t, format } = useI18n();
    const words = t.changes;
    const relative = inside(detail, file.path);
    const name = relative.slice(relative.lastIndexOf('/') + 1);
    const sizes = [file.fromSize, file.toSize]
        .filter((size) => size !== null)
        .map((size) => format.bytes(size));
    const facts = [
        file.kind === 'text' ? null : words.diff.fileKinds[file.kind],
        file.lines !== null
            ? words.lines(format.number(file.lines.added), format.number(file.lines.removed))
            : file.change === 'modified' && sizes.length === 2
              ? words.resource.sizes(sizes[0] ?? '', sizes[1] ?? '')
              : null,
        file.fromPath === null ? null : words.resources.fromPath(inside(detail, file.fromPath)),
    ].filter((fact) => fact !== null);
    const label = (
        <>
            <span className="flex min-w-0 items-center gap-1.5">
                <span className="min-w-0 font-mono text-small text-ink [overflow-wrap:anywhere]">{name}</span>
                {file.flags.length > 0 && (
                    <TriangleAlert
                        size={12}
                        strokeWidth={2}
                        className="flex-none text-warning"
                        aria-hidden="true"
                    />
                )}
            </span>
            <span
                className={cn(
                    'flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-meta group-hover:text-ink-2',
                    current ? 'text-ink-2' : 'text-ink-3',
                )}
            >
                <Chip icon={FILE_CHIP[file.change]}>{words.diff.fileChanges[file.change]}</Chip>
                {facts.join(' · ')}
            </span>
        </>
    );
    return (
        <li className="flex items-center gap-1 pr-2">
            {onPick === undefined ? (
                <div className="flex min-w-0 flex-1 flex-col gap-1 px-5 py-1.5 max-md:px-4">{label}</div>
            ) : (
                <button
                    type="button"
                    aria-current={current || undefined}
                    onClick={() => onPick(file.path)}
                    className={cn(
                        'group flex min-w-0 flex-1 flex-col gap-1 px-5 py-1.5 text-left hover:bg-hover max-md:px-4',
                        current && 'bg-accent-soft shadow-[inset_2px_0_0_var(--accent)]',
                    )}
                >
                    {label}
                </button>
            )}
            {onPurge !== undefined && (
                <Button
                    size="icon-sm"
                    tone="ghost"
                    icon={Eraser}
                    aria-label={t.changes.resource.purge(name)}
                    onClick={onPurge}
                />
            )}
        </li>
    );
}

function Purge({
    file,
    onClose,
    onPurged,
}: {
    file: FileChange;
    onClose: () => void;
    onPurged: (purged: Purged) => void;
}) {
    const { t } = useI18n();
    const queryClient = useQueryClient();
    const purge = useMutation({
        mutationFn: () => purgeContents(file.path),
        onSuccess: (purged) => {
            void queryClient.invalidateQueries({ queryKey: ['changes'] });
            onPurged(purged);
        },
    });
    return (
        <Dialog
            title={t.changes.resource.purgeTitle}
            role="alertdialog"
            onClose={onClose}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button tone="danger" pending={purge.isPending} onClick={() => purge.mutate()}>
                        {t.changes.resource.purgeConfirm}
                    </Button>
                </>
            }
        >
            <p className="font-mono text-small text-ink [overflow-wrap:anywhere]">{file.path}</p>
            <p>{t.changes.resource.purgeBody}</p>
            {purge.isError && <Notice tone="danger">{errorMessage(t, purge.error)}</Notice>}
        </Dialog>
    );
}

function HistoryCard({ name }: { name: string }) {
    const { t, format } = useI18n();
    const words = t.changes.resource;
    const history = useQuery(historyQuery(name));
    return (
        <Card id="changes-history" icon={History} title={words.history} meta={words.historyMeta}>
            {history.isError ? (
                <Notice tone="danger">{errorMessage(t, history.error)}</Notice>
            ) : history.data === undefined ? (
                <div className="skeleton h-16 w-full" aria-busy="true" aria-label={t.common.loading} />
            ) : history.data.length === 0 ? (
                <p className="text-ui text-ink-3">{words.historyEmpty}</p>
            ) : (
                <ol aria-label={words.historyCaption} className="flex flex-col gap-2">
                    {history.data.map((version) => (
                        <li
                            key={version.until}
                            className="flex flex-wrap items-center gap-x-2.5 gap-y-0.5 text-ui text-ink-2"
                        >
                            <Package size={14} strokeWidth={1.5} className="text-ink-3" aria-hidden="true" />
                            <span className="font-mono text-small text-ink">
                                {version.version ?? words.noVersion}
                            </span>
                            <span>
                                {words.since(format.timestamp(version.since.takenAt, { seconds: false }))}
                            </span>
                            <span className="font-mono text-meta text-ink-3 [overflow-wrap:anywhere]">
                                {version.path}
                            </span>
                        </li>
                    ))}
                </ol>
            )}
        </Card>
    );
}
