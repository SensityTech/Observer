import { Cpu, Download, Network, OctagonX, Trash2, TriangleAlert, type LucideIcon } from 'lucide-react';
import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';

import { deleteCrashFiles, downloadCrashFile, MAX_DOWNLOAD } from '../api/diagnostics';
import type { Crash } from '../api/generated/Crash';
import type { CrashFile } from '../api/generated/CrashFile';
import type { CrashFolder } from '../api/generated/CrashFolder';
import type { Destination } from '../api/generated/Destination';
import type { DiagnosticsSnapshot } from '../api/generated/DiagnosticsSnapshot';
import type { Frame } from '../api/generated/Frame';
import type { Listener } from '../api/generated/Listener';
import type { ProcessKind } from '../api/generated/ProcessKind';
import type { TreeProcess } from '../api/generated/TreeProcess';
import { useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Chip } from '../components/Chip';
import { Dialog } from '../components/Dialog';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { applyEvent } from '../diagnostics/store';
import { Code, exceptionWords, exitWords, threadWord } from '../diagnostics/words';
import type { Messages } from '../i18n/en';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState, useTopic } from '../live/context';
import { cn } from '../lib/cn';
import { useNow } from '../lib/useNow';

const DAY = 24 * 60 * 60 * 1000;

export function DiagnosticsPage() {
    const { t } = useI18n();
    const link = useLinkState();
    const [snapshot, setSnapshot] = useState<DiagnosticsSnapshot | null>(null);
    const [refused, setRefused] = useState(false);
    useTopic('server.diagnostics', {
        snapshot: (data) => {
            setSnapshot(data);
            setRefused(false);
        },
        event: (event) => setSnapshot((current) => (current === null ? null : applyEvent(current, event))),
        refused: () => setRefused(true),
        cleared: () => setSnapshot(null),
    });
    const updated = (crash: Crash) =>
        setSnapshot((current) => (current === null ? null : applyEvent(current, { kind: 'crash', crash })));

    let body: ReactNode;
    if (refused) {
        body = <Notice tone="warning">{t.diagnostics.forbidden}</Notice>;
    } else if (snapshot === null) {
        const waiting = link === 'live' || link === 'connecting';
        body = waiting ? (
            <div className="flex flex-col gap-5" aria-busy="true" aria-label={t.diagnostics.loading}>
                {[0, 1, 2].map((index) => (
                    <div key={index} className="skeleton h-48 w-full rounded-card" />
                ))}
            </div>
        ) : (
            <Notice>{t.diagnostics.unavailable}</Notice>
        );
    } else {
        body = (
            <>
                <CrashesCard crashes={snapshot.crashes} folder={snapshot.crashFolder} onUpdated={updated} />
                <ProcessesCard processes={snapshot.processes} />
                <ConnectionsCard destinations={snapshot.destinations} listeners={snapshot.listeners} />
            </>
        );
    }

    return (
        <>
            <div className="page-head">
                <h1 className="page-title">
                    {t.diagnostics.title} <span className="gloss">{t.diagnostics.gloss}</span>
                </h1>
            </div>
            {body}
        </>
    );
}

function Card({
    id,
    icon: Icon,
    title,
    meta,
    footer,
    children,
}: {
    id: string;
    icon: LucideIcon;
    title: string;
    meta: string;
    footer?: string;
    children: ReactNode;
}) {
    return (
        <section className="card" aria-labelledby={id}>
            <div className="card-head wrap-push">
                <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                    <Icon size={14} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <h2 id={id} className="text-card text-ink">
                    {title}
                </h2>
                <span className="text-meta text-ink-3" data-push>
                    {meta}
                </span>
            </div>
            {children}
            {footer !== undefined && (
                <p className="rounded-b-card border-t border-line bg-bg-subtle px-5 py-2.5 text-meta text-ink-3 max-md:px-4">
                    {footer}
                </p>
            )}
        </section>
    );
}

function Empty({ title, body }: { title: string; body: string }) {
    return (
        <div className="p-5 max-md:p-4">
            <div className="flex flex-col items-center gap-2 rounded-card border border-dashed border-line-strong bg-bg-subtle px-6 py-10 text-center dot-grid">
                <p className="text-card text-ink">{title}</p>
                <p className="max-w-empty text-body text-ink-3">{body}</p>
            </div>
        </div>
    );
}

const folderMeta = (t: Messages, format: ReturnType<typeof useI18n>['format'], folder: CrashFolder) =>
    [
        t.diagnostics.crashes.meta(
            format.number(folder.files),
            format.number(folder.maxFiles),
            format.bytes(folder.bytes),
            format.bytes(folder.maxBytes),
        ),
        folder.free === null ? null : t.diagnostics.crashes.free(format.bytes(folder.free)),
    ]
        .filter((part) => part !== null)
        .join(' · ');

/** What a record says happened, in one sentence. */
const crashSentence = (t: Messages, crash: Crash): ReactNode => {
    const analysis = crash.analysis;
    const module = analysis?.frames.find((frame) => frame.module !== null)?.module ?? null;
    if (crash.kind === 'found') {
        return t.diagnostics.crashes.found;
    }
    if (crash.kind === 'hang') {
        const thread = threadWord(analysis?.thread?.name ?? null);
        const main = analysis?.threads[0];
        if (main !== undefined && main.state === 'running' && (main.cpu ?? 0) >= 90) {
            return t.alerts.busy(thread);
        }
        if (main?.syscall !== null && main?.syscall !== undefined) {
            return (
                <>
                    {t.alerts.waitingIn(thread)} <Code>{main.syscall}</Code>
                </>
            );
        }
        if (module !== null) {
            return (
                <>
                    {t.alerts.stuckIn(thread)} <Code>{module}</Code>
                </>
            );
        }
        return t.diagnostics.crashes.hung;
    }
    const exception = analysis?.exception ?? null;
    if (exception !== null) {
        const words = exceptionWords(t, exception.code, exception.name);
        return module === null ? (
            words
        ) : (
            <>
                {words} {t.alerts.crashedIn} <Code>{module}</Code>
            </>
        );
    }
    const exit = crash.exitCode ?? crash.exit;
    return exit === null ? t.alerts.crashed : t.diagnostics.crashes.exited(exitWords(t, exit));
};

const CRASH_TONE = { crash: 'danger', hang: 'warning', found: 'neutral' } as const;
const CRASH_ICON = { crash: OctagonX, hang: TriangleAlert, found: undefined } as const;

function CrashesCard({
    crashes,
    folder,
    onUpdated,
}: {
    crashes: Crash[];
    folder: CrashFolder;
    onUpdated: (crash: Crash) => void;
}) {
    const { t, format } = useI18n();
    const [open, setOpen] = useState<number | null>(null);
    const [deleting, setDeleting] = useState<Crash | null>(null);
    const [deleted, setDeleted] = useState(0);
    const notice = useRef<HTMLDivElement>(null);
    // The deletion takes the row's button away with the dialog: the keyboard goes to the notice
    // saying what happened rather than back to the top of the document.
    useEffect(() => {
        if (deleted > 0 && document.activeElement === document.body) {
            notice.current?.focus();
        }
    }, [deleted]);
    return (
        <Card
            id="diagnostics-crashes"
            icon={OctagonX}
            title={t.diagnostics.crashes.title}
            meta={folderMeta(t, format, folder)}
        >
            {deleted > 0 && (
                <div className="px-5 pt-4 max-md:px-4">
                    <Notice ref={notice} tabIndex={-1} tone="success">
                        {t.diagnostics.crashes.deleted}
                    </Notice>
                </div>
            )}
            {crashes.length === 0 ? (
                <Empty title={t.diagnostics.crashes.emptyTitle} body={t.diagnostics.crashes.emptyBody} />
            ) : (
                <div className={table.scroll}>
                    <table className={table.table}>
                        <caption className="sr-only">{t.diagnostics.crashes.caption}</caption>
                        <thead>
                            <tr>
                                <th scope="col" className={table.th}>
                                    {t.diagnostics.crashes.when}
                                </th>
                                <th scope="col" className={table.th}>
                                    {t.diagnostics.crashes.what}
                                </th>
                                <th scope="col" className={table.th}>
                                    {t.diagnostics.crashes.group}
                                </th>
                                <th scope="col" className={table.th}>
                                    {t.diagnostics.crashes.files}
                                </th>
                                <th scope="col" className={table.th}>
                                    <span className="sr-only">{t.diagnostics.crashes.actions}</span>
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            {crashes.map((crash) => (
                                <CrashRow
                                    key={crash.id}
                                    crash={crash}
                                    open={open === crash.id}
                                    onToggle={() => setOpen(open === crash.id ? null : crash.id)}
                                    onDelete={() => setDeleting(crash)}
                                />
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
            {deleting !== null && (
                <DeleteFiles
                    crash={deleting}
                    onClose={() => setDeleting(null)}
                    onDeleted={(crash) => {
                        onUpdated(crash);
                        setDeleting(null);
                        setDeleted((count) => count + 1);
                    }}
                />
            )}
        </Card>
    );
}

function CrashRow({
    crash,
    open,
    onToggle,
    onDelete,
}: {
    crash: Crash;
    open: boolean;
    onToggle: () => void;
    onDelete: () => void;
}) {
    const { t, format } = useI18n();
    const me = useMe();
    const thread = crash.analysis?.thread?.name ?? null;
    const facts = [
        thread === null ? null : t.diagnostics.crashes.thread(thread),
        crash.build === null ? null : t.diagnostics.crashes.build(crash.build),
        crash.analysisState === 'pending' ? t.diagnostics.crashes.reading : null,
        crash.analysisState === 'unreadable' ? t.diagnostics.crashes.unreadable : null,
    ].filter((fact) => fact !== null);
    const kept = crash.files.some((file) => file.kept);
    const detailsId = `crash-${crash.id}-details`;
    return (
        <>
            <tr className={table.row}>
                <td className={cn(table.td, table.mono, 'align-top')}>
                    <time dateTime={crash.at}>{format.timestamp(crash.at, { seconds: false })}</time>
                </td>
                <td className={cn(table.td, 'min-w-64 align-top')}>
                    <div className="flex flex-wrap items-center gap-x-1.5 gap-y-1 text-ink">
                        <Chip tone={CRASH_TONE[crash.kind]} icon={CRASH_ICON[crash.kind]}>
                            {t.diagnostics.crashes.kinds[crash.kind]}
                        </Chip>
                        <span className="[overflow-wrap:anywhere]">{crashSentence(t, crash)}</span>
                    </div>
                    {facts.length > 0 && <div className={table.sub}>{facts.join(' · ')}</div>}
                </td>
                <td className={cn(table.td, 'align-top whitespace-nowrap')}>
                    {crash.group !== null && crash.group.count > 1
                        ? t.diagnostics.crashes.inDays(crash.group.count)
                        : ''}
                </td>
                <td className={cn(table.td, 'align-top')}>
                    <ul className="flex flex-col gap-0.5">
                        {crash.files.map((file) => (
                            <FileLine key={file.name} crash={crash} file={file} />
                        ))}
                    </ul>
                </td>
                <td className={cn(table.td, 'align-top text-right')}>
                    <div className="flex justify-end gap-1.5 whitespace-nowrap">
                        {crash.analysis !== null && (
                            <Button
                                size="sm"
                                tone="ghost"
                                aria-expanded={open}
                                aria-controls={detailsId}
                                onClick={onToggle}
                            >
                                {open ? t.diagnostics.crashes.hideDetails : t.diagnostics.crashes.details}
                            </Button>
                        )}
                        {me.isOwner && kept && (
                            <Button size="sm" tone="ghost" icon={Trash2} onClick={onDelete}>
                                {t.diagnostics.crashes.deleteFiles}
                            </Button>
                        )}
                    </div>
                </td>
            </tr>
            {open && crash.analysis !== null && (
                <tr id={detailsId}>
                    <td colSpan={5} className={cn(table.td, 'bg-bg-subtle')}>
                        <CrashDetails crash={crash} />
                    </td>
                </tr>
            )}
        </>
    );
}

function FileLine({ crash, file }: { crash: Crash; file: CrashFile }) {
    const { t, format } = useI18n();
    const me = useMe();
    const [failure, setFailure] = useState<string | null>(null);
    const [pending, setPending] = useState(false);
    const large = file.bytes > MAX_DOWNLOAD;
    const words = `${format.bytes(file.bytes)} ${t.diagnostics.crashes.fileKinds[file.kind]}`;
    const download = async () => {
        setFailure(null);
        setPending(true);
        try {
            await downloadCrashFile(crash.id, file.name);
        } catch (error) {
            setFailure(errorMessage(t, error));
        } finally {
            setPending(false);
        }
    };
    return (
        <li className="flex flex-wrap items-center gap-x-1.5">
            <span className="inline-flex items-center gap-x-1.5 whitespace-nowrap">
                {file.kept ? (
                    <span>{large ? `${words} · ${t.diagnostics.crashes.staysOnHost}` : words}</span>
                ) : (
                    <span className="text-ink-3">
                        <s>{words}</s> {t.diagnostics.crashes.removed}
                    </span>
                )}
                {me.isOwner && file.kept && !large && (
                    <Button
                        size="icon-sm"
                        tone="ghost"
                        icon={Download}
                        aria-label={t.diagnostics.crashes.download(file.name)}
                        pending={pending}
                        onClick={() => void download()}
                    />
                )}
            </span>
            {failure !== null && (
                <span className="text-meta whitespace-normal text-danger-ink">{failure}</span>
            )}
        </li>
    );
}

/** Each frame as `module+offset`, keyed apart when a stack holds the same address twice. */
const frameLines = (frames: Frame[]) => {
    const seen = new Map<string, number>();
    return frames.map((frame) => {
        const text = `${frame.module ?? '?'}+${frame.offset}`;
        const repeat = (seen.get(text) ?? 0) + 1;
        seen.set(text, repeat);
        return { key: `${text}#${repeat}`, text };
    });
};

function CrashDetails({ crash }: { crash: Crash }) {
    const { t, format } = useI18n();
    const analysis = crash.analysis;
    if (analysis === null) {
        return null;
    }
    const [main, ...rest] = analysis.threads;
    const threads = main === undefined ? [] : [main, ...rest.slice(0, 5)];
    return (
        <div className="flex flex-col gap-4 py-2">
            {analysis.frames.length > 0 && (
                <div>
                    <p className="eyebrow mb-1.5">{t.diagnostics.crashes.frames}</p>
                    <ol className="flex flex-col gap-0.5 font-mono text-small text-ink [overflow-wrap:anywhere]">
                        {frameLines(analysis.frames).map(({ key, text }) => (
                            <li key={key}>{text}</li>
                        ))}
                    </ol>
                </div>
            )}
            {crash.watchdogStack !== null && (
                <div>
                    <p className="eyebrow mb-1.5">{t.diagnostics.crashes.watchdog}</p>
                    <pre className="max-h-48 overflow-auto rounded-box border border-line bg-surface p-3 font-mono text-small whitespace-pre-wrap text-ink">
                        {crash.watchdogStack}
                    </pre>
                </div>
            )}
            {threads.length > 0 && (
                <div>
                    <p className="eyebrow mb-1.5">{t.diagnostics.crashes.threads}</p>
                    {analysis.syscallsReadable === false && (
                        <p className="mb-1.5 text-meta text-ink-3">{t.diagnostics.crashes.syscallsHidden}</p>
                    )}
                    <table className={cn(table.table, 'max-w-[640px]')}>
                        <caption className="sr-only">{t.diagnostics.crashes.threadsCaption}</caption>
                        <thead>
                            <tr>
                                <th scope="col" className={table.th}>
                                    {t.diagnostics.crashes.threadName}
                                </th>
                                <th scope="col" className={table.th}>
                                    {t.diagnostics.crashes.threadState}
                                </th>
                                <th scope="col" className={table.th}>
                                    {t.diagnostics.crashes.threadCpu}
                                </th>
                                <th scope="col" className={table.th}>
                                    {t.diagnostics.crashes.threadCall}
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            {threads.map((thread) => (
                                <tr key={thread.tid}>
                                    <td className={cn(table.td, table.mono)}>{thread.name}</td>
                                    <td className={table.td}>{t.diagnostics.threadStates[thread.state]}</td>
                                    <td className={cn(table.td, 'tabular-nums')}>
                                        {thread.cpu === null ? '' : format.percent(thread.cpu)}
                                    </td>
                                    <td className={table.td}>
                                        {thread.syscall === null ? (
                                            ''
                                        ) : (
                                            <>
                                                <Code>{thread.syscall}</Code>
                                                {thread.syscallGroup !== null &&
                                                    ` ${t.diagnostics.syscallGroups[thread.syscallGroup]}`}
                                            </>
                                        )}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </div>
    );
}

function DeleteFiles({
    crash,
    onClose,
    onDeleted,
}: {
    crash: Crash;
    onClose: () => void;
    onDeleted: (crash: Crash) => void;
}) {
    const { t } = useI18n();
    const [pending, setPending] = useState(false);
    const [failure, setFailure] = useState<string | null>(null);
    const confirm = async () => {
        setPending(true);
        setFailure(null);
        try {
            onDeleted(await deleteCrashFiles(crash.id));
        } catch (error) {
            setFailure(errorMessage(t, error));
            setPending(false);
        }
    };
    return (
        <Dialog
            title={t.diagnostics.crashes.deleteTitle}
            role="alertdialog"
            onClose={onClose}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button tone="danger" pending={pending} onClick={() => void confirm()}>
                        {t.diagnostics.crashes.deleteConfirm}
                    </Button>
                </>
            }
        >
            <p>{t.diagnostics.crashes.deleteBody}</p>
            <ul className="font-mono text-small text-ink">
                {crash.files
                    .filter((file) => file.kept)
                    .map((file) => (
                        <li key={file.name}>{file.name}</li>
                    ))}
            </ul>
            {failure !== null && <Notice tone="danger">{failure}</Notice>}
        </Dialog>
    );
}

const KIND_TONE: Record<ProcessKind, 'warning' | 'neutral'> = {
    shell: 'warning',
    other: 'neutral',
    dumpServer: 'neutral',
    consoleHost: 'neutral',
    node: 'neutral',
};

function ProcessesCard({ processes }: { processes: TreeProcess[] }) {
    const { t } = useI18n();
    return (
        <Card
            id="diagnostics-processes"
            icon={Cpu}
            title={t.diagnostics.processes.title}
            meta={t.diagnostics.processes.meta}
            footer={t.diagnostics.processes.limits}
        >
            {processes.length === 0 ? (
                <Empty title={t.diagnostics.processes.emptyTitle} body={t.diagnostics.processes.emptyBody} />
            ) : (
                <div className={table.scroll}>
                    <table className={table.table}>
                        <caption className="sr-only">{t.diagnostics.processes.caption}</caption>
                        <thead>
                            <tr>
                                {[
                                    t.diagnostics.processes.started,
                                    t.diagnostics.processes.process,
                                    t.diagnostics.processes.kind,
                                    t.diagnostics.processes.parent,
                                    t.diagnostics.processes.run,
                                    t.diagnostics.processes.count,
                                    t.diagnostics.processes.state,
                                ].map((heading) => (
                                    <th key={heading} scope="col" className={table.th}>
                                        {heading}
                                    </th>
                                ))}
                            </tr>
                        </thead>
                        <tbody>
                            {processes.map((process) => (
                                <ProcessRow key={process.id} process={process} />
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </Card>
    );
}

function ProcessRow({ process }: { process: TreeProcess }) {
    const { t, format } = useI18n();
    const [whole, setWhole] = useState(false);
    const [hidden, setHidden] = useState(false);
    const clamped = useRef<HTMLSpanElement>(null);
    const alert = process.kind === 'shell';
    const line = process.commandLine;
    // Whether two lines hide part of the command depends on the column's width, not on its length.
    useLayoutEffect(() => {
        const span = clamped.current;
        if (span === null || line === null || whole) {
            return;
        }
        const measure = () => setHidden(span.scrollHeight > span.clientHeight);
        measure();
        if (typeof ResizeObserver === 'undefined') {
            return;
        }
        const resized = new ResizeObserver(measure);
        resized.observe(span);
        return () => resized.disconnect();
    }, [line, whole]);
    return (
        <tr className={alert ? 'bg-warning-soft [&:last-child>td]:border-b-0' : table.row}>
            <td
                className={cn(
                    table.td,
                    table.mono,
                    'align-top',
                    alert && 'shadow-[inset_2px_0_0_var(--warning-fill)]',
                )}
            >
                <time dateTime={process.lastStartedAt}>
                    {format.timestamp(process.lastStartedAt, { seconds: false })}
                </time>
            </td>
            <td className={cn(table.td, 'max-w-[480px] min-w-64 align-top')}>
                <span className={cn(table.mono, '[overflow-wrap:anywhere] whitespace-normal')}>
                    {process.name ?? t.diagnostics.processes.unread}
                </span>
                {line !== null && (
                    <div className={cn(table.sub, 'block')}>
                        <span
                            ref={clamped}
                            className={cn('font-mono [overflow-wrap:anywhere]', !whole && 'line-clamp-2')}
                            title={whole ? undefined : line}
                        >
                            {line}
                        </span>
                        {process.truncated && <span> · {t.diagnostics.processes.cut}</span>}
                        {(hidden || whole) && (
                            <button
                                type="button"
                                className="mt-0.5 block font-medium text-ink-2 underline-offset-2 hover:underline max-md:min-h-11"
                                aria-expanded={whole}
                                onClick={() => setWhole(!whole)}
                            >
                                {whole ? t.diagnostics.processes.less : t.diagnostics.processes.more}
                            </button>
                        )}
                    </div>
                )}
            </td>
            <td className={cn(table.td, 'align-top')}>
                <Chip tone={KIND_TONE[process.kind]} icon={alert ? TriangleAlert : undefined}>
                    {t.diagnostics.processes.kinds[process.kind]}
                </Chip>
            </td>
            <td className={cn(table.td, 'align-top whitespace-nowrap')}>
                {process.parent === null
                    ? ''
                    : [process.parent.name, String(process.parent.pid)]
                          .filter((part) => part !== null)
                          .join(' · ')}
            </td>
            <td className={cn(table.td, 'align-top tabular-nums')}>
                {t.diagnostics.processes.generation(process.generation)}
            </td>
            <td className={cn(table.td, 'align-top tabular-nums')}>
                {t.diagnostics.processes.times(process.count)}
            </td>
            <td className={cn(table.td, 'align-top whitespace-nowrap')}>
                {process.endedAt === null
                    ? t.diagnostics.processes.running
                    : t.diagnostics.processes.ended(
                          format.dateTime(new Date(process.endedAt), { dateStyle: undefined }),
                      )}
            </td>
        </tr>
    );
}

/** An address and port as people write them: IPv6 in brackets. */
const endpoint = (address: string, port: number) =>
    address.includes(':') ? `[${address}]:${port}` : `${address}:${port}`;

function ConnectionsCard({
    destinations,
    listeners,
}: {
    destinations: Destination[];
    listeners: Listener[];
}) {
    const { t, format } = useI18n();
    const now = useNow(60_000);
    const words = t.diagnostics.connections;
    return (
        <Card
            id="diagnostics-connections"
            icon={Network}
            title={words.title}
            meta={words.meta}
            footer={words.footer}
        >
            <h3 className="px-5 pt-4 pb-2 text-ui font-semibold text-ink max-md:px-4">
                {words.destinations}
            </h3>
            {destinations.length === 0 ? (
                <p className="px-5 pb-4 text-ui text-ink-3 max-md:px-4">{words.none}</p>
            ) : (
                <div className={cn(table.scroll, 'rounded-none border-t border-line')}>
                    <table className={table.table}>
                        <caption className="sr-only">{words.destinationsCaption}</caption>
                        <thead>
                            <tr>
                                {[
                                    words.destination,
                                    words.scope,
                                    words.protocol,
                                    words.process,
                                    words.openNow,
                                    words.peak,
                                    words.lastSeen,
                                ].map((heading) => (
                                    <th key={heading} scope="col" className={table.th}>
                                        {heading}
                                    </th>
                                ))}
                            </tr>
                        </thead>
                        <tbody>
                            {destinations.map((destination) => (
                                <tr
                                    key={`${destination.protocol} ${destination.address} ${destination.port} ${destination.process}`}
                                    className={table.row}
                                >
                                    <td className={table.td}>
                                        <span className={table.mono}>
                                            {endpoint(destination.address, destination.port)}
                                        </span>
                                        {destination.service !== null && (
                                            <div className={table.sub}>
                                                {words.services[destination.service]}
                                            </div>
                                        )}
                                    </td>
                                    <td className={table.td}>
                                        <span className="flex flex-wrap gap-1.5">
                                            <Chip>{words.scopes[destination.scope]}</Chip>
                                            {now - Date.parse(destination.firstSeen) < DAY && (
                                                <Chip>{words.new}</Chip>
                                            )}
                                        </span>
                                    </td>
                                    <td className={table.td}>{words.protocols[destination.protocol]}</td>
                                    <td className={cn(table.td, table.mono)}>{destination.process}</td>
                                    <td
                                        className={cn(
                                            table.td,
                                            'w-[1%] text-right font-semibold whitespace-nowrap text-ink tabular-nums',
                                        )}
                                    >
                                        {format.number(destination.open)}
                                    </td>
                                    <td
                                        className={cn(
                                            table.td,
                                            'w-[1%] text-right whitespace-nowrap tabular-nums',
                                        )}
                                    >
                                        {format.number(destination.peak)}
                                    </td>
                                    <td className={cn(table.td, table.mono)}>
                                        {format.timestamp(destination.lastSeen, { seconds: false })}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
            <h3 className="border-t border-line px-5 pt-4 pb-2 text-ui font-semibold text-ink max-md:px-4">
                {words.listening}
            </h3>
            {listeners.length === 0 ? (
                <p className="px-5 pb-4 text-ui text-ink-3 max-md:px-4">{words.noListener}</p>
            ) : (
                <div className={cn(table.scroll, 'rounded-none border-t border-line')}>
                    <table className={table.table}>
                        <caption className="sr-only">{words.listeningCaption}</caption>
                        <thead>
                            <tr>
                                {[
                                    words.endpoint,
                                    words.protocol,
                                    words.process,
                                    words.role,
                                    words.inbound,
                                ].map((heading) => (
                                    <th key={heading} scope="col" className={table.th}>
                                        {heading}
                                    </th>
                                ))}
                            </tr>
                        </thead>
                        <tbody>
                            {listeners.map((listener) => (
                                <tr
                                    key={`${listener.protocol} ${listener.address} ${listener.port} ${listener.process}`}
                                    className={table.row}
                                >
                                    <td className={cn(table.td, table.mono)}>
                                        {endpoint(listener.address, listener.port)}
                                    </td>
                                    <td className={table.td}>{words.protocols[listener.protocol]}</td>
                                    <td className={cn(table.td, table.mono)}>{listener.process}</td>
                                    <td className={table.td}>{listener.role === 'game' ? words.game : ''}</td>
                                    <td
                                        className={cn(
                                            table.td,
                                            'w-[1%] text-right font-semibold whitespace-nowrap text-ink tabular-nums',
                                        )}
                                    >
                                        {format.number(listener.inbound)}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </Card>
    );
}
