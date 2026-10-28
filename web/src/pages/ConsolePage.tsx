import { useQuery } from '@tanstack/react-query';
import { useVirtualizer } from '@tanstack/react-virtual';
import { ArrowDown, Check, Download, Minus, Pause, Play, Search } from 'lucide-react';
import {
    useEffect,
    useLayoutEffect,
    useMemo,
    useRef,
    useState,
    useSyncExternalStore,
    type FormEvent,
    type KeyboardEvent,
} from 'react';

import { downloadLog, logFilesQuery, sendCommand } from '../api/server';
import type { LineKind } from '../api/generated/LineKind';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { newer, older, readHistory, remember, START, type Position } from '../console/history';
import type { Tone } from '../console/sgr';
import { ConsoleStore, type LineRow, type Row } from '../console/store';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { useTopic } from '../live/context';
import { cn } from '../lib/cn';
import { useServerStatus } from '../server/context';
import { isRunning } from '../server/state';

const KINDS: LineKind[] = ['stdout', 'stderr', 'command', 'observer'];
const EVERY_KIND = { stdout: true, stderr: true, command: true, observer: true } satisfies Record<
    LineKind,
    boolean
>;
/** The view follows the newest line while it is this close to the bottom. */
const BOTTOM = 6;
/** How often a screen reader hears what came in, rather than every line. */
const SUMMARY_EVERY = 15_000;

const TONE = {
    red: 'text-term-red',
    green: 'text-term-green',
    yellow: 'text-term-yellow',
    blue: 'text-term-blue',
    magenta: 'text-term-magenta',
    cyan: 'text-term-cyan',
    bright: 'text-ink',
    dim: 'text-term-dim',
} satisfies Record<Tone, string>;

const pad = (value: number) => String(value).padStart(2, '0');
const clock = (at: number) => {
    const date = new Date(at);
    return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
};

/** What the view holds still: the rows as they were when the reader paused or scrolled up. */
type Frozen = { rows: readonly Row[]; received: number };

export function ConsolePage() {
    const { t } = useI18n();
    const me = useMe();
    const [store] = useState(() => new ConsoleStore());
    const rows = useSyncExternalStore(store.subscribe, store.getRows);
    const [refused, setRefused] = useState(false);
    const [loaded, setLoaded] = useState(false);
    const [query, setQuery] = useState('');
    const [shown, setShown] = useState<Record<LineKind, boolean>>(EVERY_KIND);
    const [paused, setPaused] = useState(false);
    const [frozen, setFrozen] = useState<Frozen | null>(null);
    const [logsOpen, setLogsOpen] = useState(false);
    const [announcement, setAnnouncement] = useState('');
    const scroller = useRef<HTMLDivElement>(null);

    useTopic('console', {
        snapshot: (data) => {
            setRefused(false);
            setLoaded(true);
            store.snapshot(data);
        },
        event: (data) => store.append(data.lines),
        refused: () => setRefused(true),
        since: store.since,
    });

    const viewed = frozen?.rows ?? rows;
    const needle = query.trim().toLowerCase();
    const visible = useMemo(
        () =>
            viewed.filter((row) =>
                row.type === 'gap'
                    ? needle === ''
                    : shown[row.kind] && (needle === '' || row.search.includes(needle)),
            ),
        [viewed, shown, needle],
    );

    // The virtualizer renders the rows it measures again on its own; nothing here is memoised around it.
    // oxlint-disable-next-line react/incompatible-library
    const virtualizer = useVirtualizer({
        count: visible.length,
        getScrollElement: () => scroller.current,
        estimateSize: () => 15,
        overscan: 30,
        getItemKey: (index) => visible[index]?.seq ?? index,
    });
    const total = virtualizer.getTotalSize();

    // Following: the newest line stays in view as lines come and rows get measured.
    useLayoutEffect(() => {
        const element = scroller.current;
        if (frozen === null && element !== null) {
            element.scrollTop = element.scrollHeight;
        }
    }, [frozen, total, visible.length]);

    const atBottom = () => {
        const element = scroller.current;
        return element === null || element.scrollHeight - element.scrollTop - element.clientHeight < BOTTOM;
    };
    const follow = () => {
        setPaused(false);
        setFrozen(null);
    };
    const onScroll = () => {
        if (paused) {
            return;
        }
        if (atBottom()) {
            setFrozen(null);
        } else if (frozen === null) {
            setFrozen({ rows, received: store.received() });
        }
    };
    const togglePause = () => {
        if (paused) {
            setPaused(false);
            if (atBottom()) {
                setFrozen(null);
            }
        } else {
            setPaused(true);
            setFrozen((current) => current ?? { rows, received: store.received() });
        }
    };
    const jumpToEnd = () => {
        follow();
        scroller.current?.focus({ preventScroll: true });
    };

    const unseen = frozen === null ? 0 : store.received() - frozen.received;

    // A short summary now and then, rather than a live region a screen reader would drown in.
    const summarised = useRef({ received: 0, errors: 0 });
    useEffect(() => {
        const timer = setInterval(() => {
            const received = store.received() - summarised.current.received;
            const errors = store.errors() - summarised.current.errors;
            summarised.current = { received: store.received(), errors: store.errors() };
            if (received > 0) {
                setAnnouncement(t.console.summary(received, errors));
            }
        }, SUMMARY_EVERY);
        return () => clearInterval(timer);
    }, [store, t]);

    const canDownload = allows(me, Permission.LogsDownload);
    const empty = visible.length === 0;
    const shownLines = visible.filter((row) => row.type === 'line').length;

    return (
        <>
            <div className="page-head">
                <h1 className="page-title">
                    {t.console.title} <span className="gloss">{t.console.gloss}</span>
                </h1>
            </div>

            <section className="card console" aria-label={t.console.card}>
                <div className="console-bar">
                    <label className="console-search">
                        <span className="sr-only">{t.console.search}</span>
                        <Search size={14} strokeWidth={1.5} aria-hidden="true" />
                        <input
                            className={cn(inputClass, 'h-8 rounded-sm pl-8.5 text-ui max-md:h-11')}
                            type="search"
                            placeholder={t.console.search}
                            autoComplete="off"
                            value={query}
                            onChange={(event) => setQuery(event.target.value)}
                        />
                    </label>
                    <fieldset className="filters">
                        <legend className="sr-only">{t.console.streams}</legend>
                        {KINDS.map((kind) => (
                            <button
                                key={kind}
                                type="button"
                                className="filter"
                                aria-pressed={shown[kind]}
                                onClick={() =>
                                    setShown((current) => ({ ...current, [kind]: !current[kind] }))
                                }
                            >
                                {shown[kind] ? (
                                    <Check size={12} strokeWidth={2} aria-hidden="true" />
                                ) : (
                                    <Minus size={12} strokeWidth={2} aria-hidden="true" />
                                )}
                                {t.console.kinds[kind]}
                            </button>
                        ))}
                    </fieldset>
                    <div className="console-tools">
                        <span className="live">
                            <span className={cn('led', !paused && 'led-live')} aria-hidden="true" />
                            {paused ? t.console.paused : t.console.streaming}
                        </span>
                        <Button
                            size="sm"
                            icon={paused ? Play : Pause}
                            className="console-tool console-pause"
                            aria-pressed={paused}
                            onClick={togglePause}
                        >
                            <span className="label-wide">{paused ? t.console.resume : t.console.pause}</span>
                        </Button>
                        {canDownload && (
                            <Button
                                size="sm"
                                icon={Download}
                                className="console-tool console-download"
                                onClick={() => setLogsOpen(true)}
                            >
                                <span className="label-wide">{t.console.download}</span>
                            </Button>
                        )}
                    </div>
                </div>

                {/* Not role="log": at hundreds of lines a second a live region would drown a screen reader. */}
                <section
                    ref={scroller}
                    className="console-out"
                    aria-label={t.console.output(shownLines)}
                    // oxlint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- the keyboard scrolls the output once it can focus it
                    tabIndex={0}
                    onScroll={onScroll}
                >
                    {refused ? (
                        <p className="console-empty">{t.console.unavailable}</p>
                    ) : empty ? (
                        loaded && (
                            <p className="console-empty">
                                {rows.length === 0 ? t.console.nothingYet : t.console.empty}
                            </p>
                        )
                    ) : (
                        <div className="relative w-full" style={{ height: total }}>
                            {virtualizer.getVirtualItems().map((item) => {
                                const row = visible[item.index];
                                if (row === undefined) {
                                    return null;
                                }
                                return (
                                    <div
                                        key={item.key}
                                        ref={virtualizer.measureElement}
                                        data-index={item.index}
                                        className="absolute top-0 left-0 w-full"
                                        style={{ transform: `translateY(${item.start}px)` }}
                                    >
                                        {row.type === 'gap' ? (
                                            <p className="ln-gap">
                                                {row.reason === 'restarted'
                                                    ? t.console.restarted
                                                    : t.console.gap}
                                            </p>
                                        ) : (
                                            <ConsoleRow row={row} />
                                        )}
                                    </div>
                                );
                            })}
                        </div>
                    )}
                </section>

                <div className="console-dock">
                    {unseen > 0 && (
                        <button type="button" className="new-lines" onClick={jumpToEnd}>
                            <ArrowDown size={14} strokeWidth={1.5} aria-hidden="true" />
                            {t.console.newLines(unseen)}
                        </button>
                    )}
                    <CommandLine
                        onSent={(command) => {
                            setAnnouncement(t.console.sent(command));
                            // Its echo is worth seeing, unless the reader paused the view on purpose.
                            if (!paused) {
                                setFrozen(null);
                            }
                        }}
                    />
                </div>
                <output className="sr-only">{announcement}</output>
            </section>

            {logsOpen && <LogFilesDialog onClose={() => setLogsOpen(false)} />}
        </>
    );
}

/** The spans with where each starts in the line, which keys them. */
const spans = (row: LineRow) => {
    let at = 0;
    return row.spans.map((span) => {
        const start = at;
        at += span.text.length;
        return { span, at: start };
    });
};

function ConsoleRow({ row }: { row: LineRow }) {
    const { t } = useI18n();
    const tag =
        row.kind === 'observer' ? (
            <span className="tag tag-observer">{t.console.observerTag}</span>
        ) : row.kind === 'command' ? (
            <span className="tag tag-cmd">{row.actor}</span>
        ) : row.kind === 'stderr' ? (
            <span className="tag tag-err">{t.console.stderrTag}</span>
        ) : null;
    return (
        <div className="ln" data-kind={row.kind}>
            <span className="ln-time">{clock(row.at)}</span>
            <span className="ln-tag">{tag}</span>
            <span className="ln-text">
                {row.kind === 'command' && <span className="ln-prompt">&gt; </span>}
                {spans(row).map(({ span, at }) => (
                    <span key={at} className={cn(span.tone && TONE[span.tone], span.bold && 'font-medium')}>
                        {span.text}
                    </span>
                ))}
            </span>
        </div>
    );
}

type Feedback = { tone: 'warning' | 'danger'; text: string };

/** The command line: Enter sends, ↑ and ↓ walk the history, Escape clears. */
function CommandLine({ onSent }: { onSent: (command: string) => void }) {
    const { t } = useI18n();
    const me = useMe();
    const { status } = useServerStatus();
    const [command, setCommand] = useState('');
    const [history, setHistory] = useState(() => readHistory(me.username));
    const [position, setPosition] = useState<Position>(START);
    const [sending, setSending] = useState(false);
    const [feedback, setFeedback] = useState<Feedback | null>(null);

    if (!allows(me, Permission.ConsoleWrite)) {
        return <p className="cmd-note">{t.console.readOnly}</p>;
    }
    const running = status !== null && isRunning(status.state);

    const submit = async (event: FormEvent) => {
        event.preventDefault();
        const line = command.trim();
        if (line === '' || sending) {
            return;
        }
        setSending(true);
        try {
            const { delivery, masked } = await sendCommand(line);
            if (delivery === 'busy') {
                setFeedback({ tone: 'danger', text: t.console.busy });
                return;
            }
            setHistory(remember(me.username, history, line, masked));
            setPosition(START);
            setCommand('');
            setFeedback(delivery === 'pending' ? { tone: 'warning', text: t.console.pending } : null);
            onSent(line);
        } catch (error) {
            setFeedback({ tone: 'danger', text: errorMessage(t, error) });
        } finally {
            setSending(false);
        }
    };

    const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
        if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
            event.preventDefault();
            const [next, value] = (event.key === 'ArrowUp' ? older : newer)(history, position, command);
            setPosition(next);
            setCommand(value);
        } else if (event.key === 'Escape') {
            setPosition(START);
            setCommand('');
        }
    };

    return (
        <>
            <form className="cmd" onSubmit={(event) => void submit(event)}>
                <label className="cmd-field">
                    <span className="cmd-prompt" aria-hidden="true">
                        &gt;
                    </span>
                    <span className="sr-only">{t.console.commandLabel}</span>
                    <input
                        className="cmd-input"
                        type="text"
                        autoComplete="off"
                        autoCapitalize="off"
                        autoCorrect="off"
                        spellCheck={false}
                        disabled={!running}
                        placeholder={running ? undefined : t.console.stopped}
                        aria-busy={sending || undefined}
                        value={command}
                        onChange={(event) => {
                            setCommand(event.target.value);
                            setPosition(START);
                            setFeedback(null);
                        }}
                        onKeyDown={onKeyDown}
                    />
                </label>
                <span className="cmd-hints">
                    <span>
                        <kbd className="kbd">Enter</kbd>
                        <span className="label-wide">{t.console.send}</span>
                    </span>
                    <span className="label-wide">
                        <kbd className="kbd">↑</kbd>
                        {t.console.history}
                    </span>
                </span>
            </form>
            {feedback && (
                <p className="cmd-feedback" data-tone={feedback.tone} role="alert">
                    {feedback.text}
                </p>
            )}
        </>
    );
}

function LogFilesDialog({ onClose }: { onClose: () => void }) {
    const { t, format } = useI18n();
    const files = useQuery(logFilesQuery);
    const [failure, setFailure] = useState<string | null>(null);
    const [downloading, setDownloading] = useState<string | null>(null);

    const download = async (name: string) => {
        setFailure(null);
        setDownloading(name);
        try {
            await downloadLog(name);
        } catch (error) {
            setFailure(errorMessage(t, error));
        } finally {
            setDownloading(null);
        }
    };

    return (
        <Dialog
            title={t.console.logs.title}
            onClose={onClose}
            className="max-w-[640px]"
            footer={
                <Button tone="ghost" onClick={onClose}>
                    {t.common.close}
                </Button>
            }
        >
            <p>{t.console.logs.body}</p>
            {failure && <Notice tone="danger">{failure}</Notice>}
            {files.isError && (
                <Notice tone="danger">
                    {t.console.logs.loadFailed} {errorMessage(t, files.error)}
                </Notice>
            )}
            {files.isPending && (
                <div className="flex flex-col gap-3" aria-busy="true">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-2/3" />
                    <span className="skeleton w-1/2" />
                </div>
            )}
            {files.data?.length === 0 && <p className="text-ink-3">{t.console.logs.none}</p>}
            {files.data && files.data.length > 0 && (
                <div className={cn(table.scroll, '-mx-5 rounded-none border-t border-line')}>
                    <table className={table.table}>
                        <caption className="sr-only">{t.console.logs.caption}</caption>
                        <thead>
                            <tr>
                                <th scope="col" className={table.th}>
                                    {t.console.logs.name}
                                </th>
                                <th scope="col" className={table.th}>
                                    {t.console.logs.size}
                                </th>
                                <th scope="col" className={table.th}>
                                    {t.console.logs.modified}
                                </th>
                                <th scope="col" className={table.th}>
                                    <span className="sr-only">{t.console.logs.actions}</span>
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            {files.data.map((file) => (
                                <tr key={file.name} className={table.row}>
                                    <td className={cn(table.td, table.mono)}>{file.name}</td>
                                    <td className={cn(table.td, 'tabular-nums')}>
                                        {format.bytes(file.bytes)}
                                    </td>
                                    <td className={cn(table.td, 'tabular-nums')}>
                                        {format.timestamp(file.modifiedAt)}
                                    </td>
                                    <td className={cn(table.td, 'text-right')}>
                                        <Button
                                            size="icon-sm"
                                            tone="ghost"
                                            icon={Download}
                                            aria-label={t.console.logs.download(file.name)}
                                            pending={downloading === file.name}
                                            onClick={() => void download(file.name)}
                                        />
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </Dialog>
    );
}
