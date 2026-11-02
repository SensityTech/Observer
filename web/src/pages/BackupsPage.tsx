import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Check, Clock, Cloud, Database, Download, History, Pencil, Plus, Trash2, X } from 'lucide-react';
import { useEffect, useId, useRef, useState, type ReactNode } from 'react';

import {
    backUpNow,
    backupRunsQuery,
    backupsQuery,
    cancelBackup,
    copiesOf,
    deleteDestination,
    downloadCopy,
    downloadable,
    kindOf,
    placeOf,
    tookSeconds,
} from '../api/backups';
import type { BackupDestination } from '../api/generated/BackupDestination';
import type { BackupRun } from '../api/generated/BackupRun';
import type { BackupsView } from '../api/generated/BackupsView';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { DestinationDialog } from '../backups/DestinationDialog';
import { KIND_ICONS } from '../backups/icons';
import { WhatDialog, WhenDialog } from '../backups/SettingsDialogs';
import { useBackupsSummary, useServerTime } from '../backups/summary';
import { Button, buttonVariants } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { Dialog } from '../components/Dialog';
import { Menu } from '../components/Menu';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

const HISTORY_PAGE = 20;

/** The game database's backups: when they run, what they dump, where the copies go, and each run. */
export function BackupsPage() {
    const { t } = useI18n();
    const backups = useQuery(backupsQuery);

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            {backups.isPending && (
                <div className="card flex flex-col gap-3 p-5">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-3/4" />
                    <span className="skeleton w-2/3" />
                    <span className="skeleton w-1/2" />
                </div>
            )}
            {backups.isError && backups.data === undefined && (
                <Notice tone="danger">
                    {t.backups.loadFailed} {errorMessage(t, backups.error)}
                </Notice>
            )}
            {backups.data !== undefined && <Backups view={backups.data} />}
        </div>
    );
}

function Backups({ view }: { view: BackupsView }) {
    const { t, format } = useI18n();
    const words = t.backups;
    const me = useMe();
    const queryClient = useQueryClient();
    const [notice, setNotice] = useState<{ tone: 'success' | 'danger'; message: string } | null>(null);
    const noticeRef = useRef<HTMLDivElement>(null);
    const refresh = () => void queryClient.invalidateQueries({ queryKey: ['backups'] });

    // The history is read again once the run under way ends, with its copies.
    const running = view.running?.run ?? null;
    const seen = useRef(running);
    useEffect(() => {
        if (seen.current !== null && seen.current !== running) {
            void queryClient.invalidateQueries({ queryKey: ['backups'] });
        }
        seen.current = running;
    }, [running, queryClient]);

    useEffect(() => {
        if (notice !== null && document.activeElement === document.body) {
            noticeRef.current?.focus();
        }
    }, [notice]);

    const start = useMutation({
        mutationFn: backUpNow,
        onMutate: () => setNotice(null),
        onSuccess: refresh,
        onError: (error) => setNotice({ tone: 'danger', message: errorMessage(t, error) }),
    });
    const cancel = useMutation({
        mutationFn: cancelBackup,
        onSuccess: refresh,
        onError: (error) => setNotice({ tone: 'danger', message: errorMessage(t, error) }),
    });

    const enabled = view.destinations.filter((destination) => destination.enabled);
    const { sentence, next } = useBackupsSummary(view);
    const canRun = allows(me, Permission.BackupsRun);

    return (
        <>
            <div className="page-head flex flex-wrap items-end justify-between gap-3">
                <div className="flex min-w-0 flex-col gap-1">
                    <h1 className="page-title">
                        {words.title} <span className="gloss">{words.gloss}</span>
                    </h1>
                    <p className="text-small text-ink-2">
                        {sentence} {next}
                    </p>
                </div>
                {canRun &&
                    (view.running === null ? (
                        <Button
                            tone="ink"
                            icon={Database}
                            pending={start.isPending}
                            disabled={enabled.length === 0}
                            title={enabled.length === 0 ? words.noDestination : undefined}
                            onClick={() => start.mutate()}
                        >
                            {words.backUpNow}
                        </Button>
                    ) : (
                        <Button icon={X} pending={cancel.isPending} onClick={() => cancel.mutate()}>
                            {words.cancel}
                        </Button>
                    ))}
            </div>
            {view.running !== null && (
                <Notice live={false}>
                    {words.progress(
                        words.phases[view.running.phase],
                        format.bytes(view.running.sqlBytes),
                        view.destinations.find((destination) => destination.id === view.running?.destination)
                            ?.name ?? null,
                        format.bytes(view.running.copiedBytes),
                    )}
                </Notice>
            )}
            {notice !== null && (
                <Notice ref={noticeRef} tabIndex={-1} tone={notice.tone}>
                    {notice.message}
                </Notice>
            )}
            <div className="grid grid-cols-2 gap-3 max-lg:grid-cols-1">
                <WhenCard view={view} />
                <WhatCard view={view} />
            </div>
            <DestinationsCard view={view} />
            <HistoryCard view={view} />
        </>
    );
}

/** One line of the When card: a check for what happens, nothing for what does not. */
function Item({ on, children, sub }: { on: boolean; children: ReactNode; sub?: ReactNode }) {
    return (
        <li className={cn('flex items-start gap-2.5', !on && 'text-ink-3')}>
            <span className="mt-0.75 flex size-3.5 flex-none text-success">
                {on && <Check size={14} strokeWidth={2} aria-hidden="true" />}
            </span>
            <span className="flex min-w-0 flex-col">
                <span className={on ? 'text-ink' : undefined}>{children}</span>
                {sub !== undefined && <span className="text-small text-ink-3">{sub}</span>}
            </span>
        </li>
    );
}

function WhenCard({ view }: { view: BackupsView }) {
    const { t, format, locale } = useI18n();
    const words = t.backups.when;
    const me = useMe();
    const [editing, setEditing] = useState(false);
    const { beforeRestarts, restartWaitSeconds, times } = view.settings;
    const wait = restartWaitSeconds === 0 ? words.noWait : words.waits(format.duration(restartWaitSeconds));

    return (
        <Card
            id="backups-when"
            icon={Clock}
            title={words.title}
            aside={
                me.isOwner && (
                    <Button size="sm" tone="ghost" onClick={() => setEditing(true)}>
                        {words.edit}
                    </Button>
                )
            }
        >
            <ul className="flex flex-col gap-3 text-body">
                {beforeRestarts === 'off' ? (
                    <Item on={false}>{words.off}</Item>
                ) : (
                    <Item on sub={wait}>
                        {beforeRestarts === 'scheduled' ? words.scheduled : words.all}
                    </Item>
                )}
                {times.length === 0 ? (
                    <Item on={false}>{words.noTimes}</Item>
                ) : (
                    <Item on sub={view.zone}>
                        {words.every}{' '}
                        <span className="font-mono">
                            {new Intl.ListFormat(locale, { type: 'conjunction' }).format(times)}
                        </span>
                    </Item>
                )}
                {beforeRestarts === 'scheduled' && <Item on={false}>{words.notManual}</Item>}
            </ul>
            {editing && <WhenDialog view={view} onClose={() => setEditing(false)} />}
        </Card>
    );
}

/** The tool's own name, out of its path: `…\bin\mariadb-dump.exe` is `mariadb-dump`. */
const toolName = (path: string) => (path.split(/[\\/]/).pop() ?? path).replace(/\.exe$/i, '');

function WhatCard({ view }: { view: BackupsView }) {
    const { t } = useI18n();
    const words = t.backups.what;
    const me = useMe();
    const [editing, setEditing] = useState(false);
    const { source, client, settings } = view;
    const place = source.socket ?? (source.host === null ? null : `${source.host}:${source.port ?? 3306}`);

    return (
        <Card
            id="backups-what"
            icon={Database}
            title={words.title}
            aside={
                me.isOwner && (
                    <Button size="sm" tone="ghost" onClick={() => setEditing(true)}>
                        {words.edit}
                    </Button>
                )
            }
        >
            <dl className="grid grid-cols-[auto_1fr] gap-x-4.5 gap-y-2.5 text-ui">
                <dt className="text-ink-3">{words.database}</dt>
                <dd className="min-w-0 [overflow-wrap:anywhere]">
                    {source.problem === null ? (
                        <span className="font-mono text-ink">{source.database}</span>
                    ) : (
                        <span className="text-warning">{t.backups.problems[source.problem]}</span>
                    )}
                </dd>
                {place !== null && (
                    <>
                        <dt className="text-ink-3">{words.server}</dt>
                        <dd className="min-w-0 font-mono text-ink [overflow-wrap:anywhere]">{place}</dd>
                    </>
                )}
                <dt className="text-ink-3">{words.engine}</dt>
                <dd className="min-w-0">
                    <span className="text-ink">{words.engines[settings.engine]}</span>{' '}
                    {client.problem === null ? (
                        <span className="text-ink-3">
                            · {words.tool(toolName(client.path ?? ''), client.version ?? '')}
                        </span>
                    ) : (
                        <span className="text-warning">· {t.backups.problems[client.problem]}</span>
                    )}
                </dd>
                <dt className="text-ink-3">{words.connection}</dt>
                <dd className="min-w-0">
                    {settings.source === 'cfg' ? (
                        <>
                            <span className="text-ink">{words.fromCfg}</span>{' '}
                            <span className="text-ink-3">
                                · <span className="font-mono">mysql_connection_string</span>
                            </span>
                        </>
                    ) : (
                        <>
                            <span className="text-ink">{words.typedIn}</span>
                            {source.user !== null && (
                                <span className="text-ink-3">
                                    {' '}
                                    · <span className="font-mono">{source.user}</span>
                                </span>
                            )}
                        </>
                    )}
                </dd>
                <dt className="text-ink-3">{words.file}</dt>
                <dd className="text-ink-2">{words.fileValue}</dd>
            </dl>
            {editing && <WhatDialog view={view} onClose={() => setEditing(false)} />}
        </Card>
    );
}

function DestinationsCard({ view }: { view: BackupsView }) {
    const { t } = useI18n();
    const words = t.backups.destinations;
    const me = useMe();
    const queryClient = useQueryClient();
    const at = useServerTime(view.zone);
    const [dialog, setDialog] = useState<{ destination: BackupDestination | null } | null>(null);
    const [deleting, setDeleting] = useState<BackupDestination | null>(null);
    const [notice, setNotice] = useState<string | null>(null);
    const remove = useMutation({
        mutationFn: (destination: BackupDestination) => deleteDestination(destination.id),
        onSuccess: () => {
            setDeleting(null);
            void queryClient.invalidateQueries({ queryKey: ['backups'] });
        },
        onError: (error) => setNotice(errorMessage(t, error)),
    });

    return (
        <Card
            id="backups-destinations"
            icon={Cloud}
            title={words.title}
            flush
            aside={
                me.isOwner && (
                    <Button size="sm" icon={Plus} onClick={() => setDialog({ destination: null })}>
                        {words.add}
                    </Button>
                )
            }
        >
            {notice !== null && (
                <div className="px-5 pt-4 max-md:px-4">
                    <Notice tone="danger">{notice}</Notice>
                </div>
            )}
            {view.destinations.length === 0 ? (
                <p className="px-6 py-4 text-center text-ui text-ink-3">{words.none}</p>
            ) : (
                <ul>
                    {view.destinations.map((destination) => (
                        <DestinationRow
                            key={destination.id}
                            destination={destination}
                            at={at}
                            owner={me.isOwner}
                            onEdit={() => setDialog({ destination })}
                            onDelete={() => setDeleting(destination)}
                        />
                    ))}
                </ul>
            )}
            {dialog !== null && (
                <DestinationDialog
                    destination={dialog.destination}
                    onClose={() => setDialog(null)}
                    onSaved={() => void queryClient.invalidateQueries({ queryKey: ['backups'] })}
                />
            )}
            {deleting !== null && (
                <Dialog
                    role="alertdialog"
                    title={words.deleteTitle(deleting.name)}
                    onClose={() => setDeleting(null)}
                    footer={
                        <>
                            <Button tone="ghost" onClick={() => setDeleting(null)}>
                                {t.common.cancel}
                            </Button>
                            <Button
                                tone="danger"
                                pending={remove.isPending}
                                onClick={() => remove.mutate(deleting)}
                            >
                                {words.deleteConfirm}
                            </Button>
                        </>
                    }
                >
                    <p>{words.deleteBody(deleting.stored)}</p>
                </Dialog>
            )}
        </Card>
    );
}

function DestinationRow({
    destination,
    at,
    owner,
    onEdit,
    onDelete,
}: {
    destination: BackupDestination;
    at: (instant: string) => string;
    owner: boolean;
    onEdit: () => void;
    onDelete: () => void;
}) {
    const { t } = useI18n();
    const words = t.backups.destinations;
    const kind = kindOf(destination.target);
    const Icon = KIND_ICONS[kind];
    const { keepLast, keepDays } = destination.retention;
    const keeps =
        keepLast !== null && keepDays !== null
            ? words.keepsBoth(keepLast, keepDays)
            : keepLast !== null
              ? words.keepsLast(keepLast)
              : words.keepsDays(keepDays ?? 0);
    const last = destination.last;
    const untrusted = destination.target.kind === 'sftp' && destination.target.hostKey === null;
    const failed = destination.enabled && (untrusted || last?.state === 'failed');
    const state = (() => {
        if (!destination.enabled) {
            return { led: 'led-off', word: words.off };
        }
        if (untrusted) {
            return { led: 'led-down', word: words.notTrusted };
        }
        if (last === null) {
            return { led: 'led-off', word: words.noCopy };
        }
        if (last.state === 'failed') {
            const problem = last.error === null ? words.failed : t.backups.short[last.error];
            return { led: 'led-down', word: `${words.failed} · ${problem}` };
        }
        if (last.state === 'uploading') {
            return { led: 'led-pending', word: words.copying };
        }
        return { led: 'led-ok', word: `${words.ok} · ${at(last.at)}` };
    })();

    return (
        <li
            className={cn(
                'flex flex-wrap items-center gap-x-3.5 gap-y-2 border-t border-line px-5 py-3.5 first:border-t-0 max-md:px-4',
                failed && 'bg-danger-soft',
            )}
        >
            <span className="grid size-6 flex-none place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                <Icon size={14} strokeWidth={1.5} aria-hidden="true" />
            </span>
            <span className="flex min-w-0 flex-[1_1_16rem] flex-col">
                <span className="flex flex-wrap items-center gap-1.5 font-medium text-ink">
                    {destination.name}
                    {(kind === 's3' || kind === 'sftp') && <Chip>{words.chips[kind]}</Chip>}
                </span>
                <span
                    className={cn('truncate font-mono text-meta', failed ? 'text-ink-2' : 'text-ink-3')}
                    title={placeOf(destination.target)}
                >
                    {placeOf(destination.target)}
                </span>
            </span>
            <span className={cn('w-40 text-small max-md:w-auto', failed ? 'text-ink-2' : 'text-ink-3')}>
                {keeps}
            </span>
            <span
                className={cn(
                    'flex w-52 items-center gap-1.5 text-small max-md:w-auto',
                    failed ? 'text-danger-ink' : 'text-ink-2',
                )}
                title={last?.errorDetail ?? undefined}
            >
                <span className={cn('led', state.led)} aria-hidden="true" />
                {state.word}
            </span>
            {owner && (
                <span className="ml-auto flex w-24 items-center justify-end gap-1 max-md:w-auto">
                    {failed && (
                        <Button size="sm" onClick={onEdit}>
                            {words.fix}
                        </Button>
                    )}
                    <Menu
                        label={words.options(destination.name)}
                        buttonClassName={buttonVariants({ size: 'icon-sm', tone: 'ghost' })}
                        placement="below-end"
                        items={[
                            { label: words.edit, icon: Pencil, onSelect: onEdit },
                            { label: words.delete, icon: Trash2, onSelect: onDelete, danger: true },
                        ]}
                    >
                        <span aria-hidden="true">···</span>
                    </Menu>
                </span>
            )}
        </li>
    );
}

function HistoryCard({ view }: { view: BackupsView }) {
    const { t, format } = useI18n();
    const words = t.backups.history;
    const me = useMe();
    const id = useId();
    const [limit, setLimit] = useState(HISTORY_PAGE);
    const runs = useQuery({
        ...backupRunsQuery(limit),
        refetchInterval: view.running === null ? false : 2000,
    });
    const [failure, setFailure] = useState<string | null>(null);
    const download = useMutation({
        mutationFn: (run: BackupRun) => {
            const copy = downloadable(run);
            return copy === null
                ? Promise.resolve()
                : downloadCopy(copy, run.fileName ?? `backup-${run.id}.sql.gz`);
        },
        onMutate: () => setFailure(null),
        onError: (error) => setFailure(errorMessage(t, error)),
    });
    const mayDownload = allows(me, Permission.BackupsDownload);
    const page = runs.data;

    const why = (run: BackupRun) => {
        switch (run.trigger) {
            case 'schedule':
                return words.why.schedule;
            case 'restart':
                return run.actor === null ? words.why.restart : words.why.restartBy(run.actor);
            case 'manual':
                return words.why.manual(run.actor ?? '');
        }
    };
    const copies = (run: BackupRun) => {
        if (run.outcome === null) {
            return <Chip>{words.running}</Chip>;
        }
        if (run.outcome === 'cancelled' || run.outcome === 'interrupted' || run.outcome === 'failed') {
            return (
                <Chip tone={run.outcome === 'failed' ? 'danger' : 'warning'}>
                    {words.outcomes[run.outcome]}
                    {run.error !== null && run.outcome === 'failed' && ` · ${t.backups.short[run.error]}`}
                </Chip>
            );
        }
        const { stored, total } = copiesOf(run);
        return <Chip tone={stored === total ? 'success' : 'warning'}>{words.copies(stored, total)}</Chip>;
    };

    return (
        <Card id={id} icon={History} title={words.title} meta={words.meta(view.runsLast30Days)} flush>
            {failure !== null && (
                <div className="px-5 pt-4 max-md:px-4">
                    <Notice tone="danger">{failure}</Notice>
                </div>
            )}
            {page === undefined ? (
                <div className="flex flex-col gap-3 p-5" aria-busy="true">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-2/3" />
                    <span className="skeleton w-1/2" />
                </div>
            ) : page.runs.length === 0 ? (
                <p className="px-6 py-4 text-center text-ui text-ink-3">{words.none}</p>
            ) : (
                <div className={table.scroll}>
                    <table className={table.table}>
                        <caption className="sr-only">{words.caption}</caption>
                        <thead>
                            <tr>
                                <th scope="col" className={table.th}>
                                    {words.when}
                                </th>
                                <th scope="col" className={table.th}>
                                    {words.whyHead}
                                </th>
                                <th scope="col" className={cn(table.th, 'text-right')}>
                                    {words.size}
                                </th>
                                <th scope="col" className={cn(table.th, 'text-right')}>
                                    {words.took}
                                </th>
                                <th scope="col" className={table.th}>
                                    {words.copiesHead}
                                </th>
                                <th scope="col" className={table.th}>
                                    <span className="sr-only">{words.actions}</span>
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            {page.runs.map((run) => {
                                const took = tookSeconds(run);
                                const copy = downloadable(run);
                                return (
                                    <tr key={run.id} className={table.row}>
                                        <td className={table.td}>
                                            <span className={table.mono}>
                                                {format.timestamp(run.startedAt, { seconds: false })}
                                            </span>
                                            {(run.serverStarted || run.looseTableCount > 0) && (
                                                <span className={table.sub}>
                                                    {run.serverStarted && words.serverStarted}
                                                    {run.serverStarted && run.looseTableCount > 0 && ' · '}
                                                    {run.looseTableCount > 0 &&
                                                        words.loose(run.looseTableCount)}
                                                </span>
                                            )}
                                        </td>
                                        <td className={cn(table.td, 'whitespace-nowrap')}>{why(run)}</td>
                                        <td className={cn(table.td, 'text-right whitespace-nowrap')}>
                                            {run.fileBytes === null ? '' : format.bytes(run.fileBytes)}
                                        </td>
                                        <td
                                            className={cn(table.td, 'text-right font-mono whitespace-nowrap')}
                                        >
                                            {took === null ? '' : format.duration(Math.round(took))}
                                        </td>
                                        <td className={table.td}>{copies(run)}</td>
                                        <td className={cn(table.td, 'text-right')}>
                                            {mayDownload && copy !== null && (
                                                <Button
                                                    size="sm"
                                                    tone="ghost"
                                                    icon={Download}
                                                    pending={
                                                        download.isPending &&
                                                        download.variables?.id === run.id
                                                    }
                                                    aria-label={words.downloadOf(
                                                        format.timestamp(run.startedAt, { seconds: false }),
                                                    )}
                                                    onClick={() => download.mutate(run)}
                                                >
                                                    <span className="max-md:sr-only">{words.download}</span>
                                                </Button>
                                            )}
                                        </td>
                                    </tr>
                                );
                            })}
                        </tbody>
                    </table>
                </div>
            )}
            {page?.nextBefore != null && (
                <div className="flex justify-center border-t border-line p-3">
                    <Button
                        size="sm"
                        tone="ghost"
                        pending={runs.isFetching}
                        onClick={() => setLimit(limit + HISTORY_PAGE)}
                    >
                        {words.more}
                    </Button>
                </div>
            )}
        </Card>
    );
}
