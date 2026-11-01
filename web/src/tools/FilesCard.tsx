import { FileCog, Undo2 } from 'lucide-react';
import { useId, useState, type Ref } from 'react';

import type { ToolChange } from '../api/generated/ToolChange';
import type { ToolUndoScope } from '../api/generated/ToolUndoScope';
import type { ToolView } from '../api/generated/ToolView';
import { hashText, scopeKey } from '../api/tools';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { CopyValue } from '../components/CopyValue';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

/** Entries of a file, and closed changes, shown before "Show all". */
const SHORT_LIST = 5;
const SHORT_CLOSED = 10;

export type Outcome = { tone: 'success' | 'warning' | 'danger'; message: string };

/** The runs the open changes hold entries of, the latest first, with how much each took out. */
const runsOf = (open: ToolChange[]) => {
    const runs = new Map<number, { files: Set<number>; removals: number }>();
    for (const change of open) {
        for (const removal of change.removals) {
            const run = runs.get(removal.run) ?? { files: new Set<number>(), removals: 0 };
            run.files.add(change.id);
            run.removals += 1;
            runs.set(removal.run, run);
        }
    }
    return [...runs.entries()]
        .map(([id, run]) => ({ id, files: run.files.size, removals: run.removals }))
        .toSorted((a, b) => b.id - a.id);
};

/**
 * The files the tool changed and still holds open, each with the copies taken out and the backup of
 * the file as it was, undone by file, by run or all at once; then those closed recently. An undo by
 * file or by run works in the folder the change was made in, all at once in the launch settings'
 * folder; a change whose folder is gone or replaced is listed without an undo.
 */
export function FilesCard({
    view,
    blocked,
    busy,
    pending,
    notice,
    noticeRef,
    onUndo,
}: {
    view: ToolView;
    /** Why nothing can be undone now: FXServer runs, or its state is not known. */
    blocked: string | null;
    busy: boolean;
    /** The scope of the undo under way, by its key. */
    pending: string | null;
    notice: Outcome | null;
    noticeRef: Ref<HTMLDivElement>;
    onUndo: (scope: ToolUndoScope) => void;
}) {
    const { t, format } = useI18n();
    const words = t.tools.changes;
    const id = useId();
    const { open, closed } = view;
    const elsewhere = new Set(view.elsewhere);
    const unreachable = new Set(view.unreachable);
    // Undo all puts back what the launch settings' folder holds, as the command line's does.
    const here = open.filter((change) => !elsewhere.has(change.id));
    const runs = runsOf(open.filter((change) => !unreachable.has(change.id)));
    // The server folder of each file only matters once changes span more than one, or lie elsewhere.
    const folders = elsewhere.size > 0 || new Set(open.map((change) => change.folder)).size > 1;
    const disabled = blocked !== null || busy;

    return (
        <Card
            id={`${id}-changes`}
            icon={FileCog}
            title={words.title}
            meta={open.length + closed.length > 0 ? words.backups(format.bytes(view.backupBytes)) : undefined}
            aside={
                here.length > 0 ? (
                    <Button
                        size="sm"
                        icon={Undo2}
                        disabled={disabled}
                        pending={pending === scopeKey('all')}
                        aria-describedby={blocked === null ? undefined : `${id}-blocked`}
                        onClick={() => onUndo('all')}
                    >
                        {words.undoAll}
                    </Button>
                ) : undefined
            }
            flush
        >
            {(notice !== null || (open.length > 0 && blocked !== null) || elsewhere.size > 0) && (
                <div className="flex flex-col gap-2 border-b border-line px-5 py-3 max-md:px-4">
                    {notice !== null && (
                        <Notice ref={noticeRef} tabIndex={-1} tone={notice.tone}>
                            {notice.message}
                        </Notice>
                    )}
                    {open.length > 0 && blocked !== null && (
                        <p id={`${id}-blocked`} className="text-small text-ink-3">
                            {blocked}
                        </p>
                    )}
                    {elsewhere.size > 0 && <p className="text-small text-ink-3">{words.elsewhere}</p>}
                </div>
            )}
            {open.length === 0 ? (
                <p className="px-6 py-6 text-center text-ui text-ink-3">{words.none}</p>
            ) : (
                <>
                    {(runs.length > 1 || (runs.length > 0 && elsewhere.size > 0)) && (
                        <section
                            aria-label={words.runs}
                            className="flex flex-col gap-1.5 border-b border-line px-5 py-3 max-md:px-4"
                        >
                            <h3 className="text-small font-medium text-ink">{words.runs}</h3>
                            <ul className="flex flex-col gap-1">
                                {runs.map((run) => (
                                    <li
                                        key={run.id}
                                        className="flex flex-wrap items-center gap-x-3 gap-y-1 text-small text-ink-2"
                                    >
                                        <span className="mr-auto">
                                            {words.run(run.id, run.files, run.removals)}
                                        </span>
                                        <Button
                                            tone="ghost"
                                            size="sm"
                                            icon={Undo2}
                                            disabled={disabled}
                                            pending={pending === scopeKey({ run: run.id })}
                                            onClick={() => onUndo({ run: run.id })}
                                        >
                                            {words.undoRun(run.id)}
                                        </Button>
                                    </li>
                                ))}
                            </ul>
                        </section>
                    )}
                    <div className={cn(table.scroll, closed.length > 0 && 'rounded-none')}>
                        <table className={table.table}>
                            <caption className="sr-only">{words.caption}</caption>
                            <thead>
                                <tr>
                                    <th scope="col" className={table.th}>
                                        {words.file}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {words.entries}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {words.backup}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {words.changed}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        <span className="sr-only">{words.actions}</span>
                                    </th>
                                </tr>
                            </thead>
                            <tbody>
                                {open.map((change) => (
                                    <tr key={change.id} className={table.row}>
                                        <td className={cn(table.td, 'min-w-56 align-top')}>
                                            <span
                                                className={cn(
                                                    table.mono,
                                                    'whitespace-normal [overflow-wrap:anywhere]',
                                                )}
                                            >
                                                {change.path}
                                            </span>
                                            <div className={table.sub}>
                                                <span className="font-mono">{change.resource}</span>
                                                <span>{words.formats[change.format]}</span>
                                                {change.state === 'pending' && <Chip>{words.pending}</Chip>}
                                                {folders && (
                                                    <span className="font-mono [overflow-wrap:anywhere]">
                                                        {change.folder}
                                                    </span>
                                                )}
                                            </div>
                                            {unreachable.has(change.id) && (
                                                <p
                                                    id={`${id}-unreachable-${change.id}`}
                                                    className="mt-1 text-small whitespace-normal text-ink-2"
                                                >
                                                    {words.unreachable}
                                                </p>
                                            )}
                                        </td>
                                        <td className={cn(table.td, 'min-w-64 align-top')}>
                                            <Removals change={change} />
                                        </td>
                                        <td className={cn(table.td, 'align-top')}>
                                            <CopyValue
                                                value={change.backup}
                                                shown={change.backup.slice(0, 12)}
                                                label={words.copyBackup(change.path)}
                                            />
                                        </td>
                                        <td className={cn(table.td, table.mono, 'align-top')}>
                                            {change.changedAt === null ? (
                                                <span className="font-sans text-ink-3">{words.notYet}</span>
                                            ) : (
                                                format.timestamp(change.changedAt)
                                            )}
                                        </td>
                                        <td className={cn(table.td, 'align-top')}>
                                            <Button
                                                tone="ghost"
                                                size="sm"
                                                icon={Undo2}
                                                disabled={disabled || unreachable.has(change.id)}
                                                pending={pending === scopeKey({ changes: [change.id] })}
                                                aria-label={words.undoFile(change.path)}
                                                aria-describedby={
                                                    unreachable.has(change.id)
                                                        ? `${id}-unreachable-${change.id}`
                                                        : undefined
                                                }
                                                onClick={() => onUndo({ changes: [change.id] })}
                                            >
                                                {words.undo}
                                            </Button>
                                        </td>
                                    </tr>
                                ))}
                            </tbody>
                        </table>
                    </div>
                </>
            )}
            {closed.length > 0 && <Closed changes={closed} />}
        </Card>
    );
}

/** The copies a file holds without, each with the copy that stands in for it. */
function Removals({ change }: { change: ToolChange }) {
    const { t } = useI18n();
    const words = t.tools.changes;
    const [all, setAll] = useState(false);
    const removals = change.removals.toSorted((a, b) => a.index - b.index);
    const shown = all ? removals : removals.slice(0, SHORT_LIST);
    return (
        <div className="flex flex-col gap-1">
            <ul className="flex flex-col gap-1">
                {shown.map((removal) => (
                    <li key={removal.index} className="flex flex-col">
                        <span className="font-mono text-small text-ink">
                            #{removal.index} {removal.name ?? hashText(removal.hash)}
                        </span>
                        <span className="font-mono text-meta text-ink-3 [overflow-wrap:anywhere]">
                            {removal.keeper.path === change.path
                                ? words.keptHere(removal.keeper.index)
                                : words.keptIn(removal.keeper.path, removal.keeper.index)}
                        </span>
                    </li>
                ))}
            </ul>
            {removals.length > SHORT_LIST && (
                <button
                    type="button"
                    className="self-start text-small font-medium text-ink-2 underline-offset-2 hover:text-ink hover:underline max-md:min-h-11"
                    aria-expanded={all}
                    onClick={() => setAll(!all)}
                >
                    {all ? words.showFewer : words.showAll(removals.length)}
                </button>
            )}
        </div>
    );
}

/** The changes closed in the last 30 days, the latest first, with why each one closed. */
function Closed({ changes }: { changes: ToolChange[] }) {
    const { t, format } = useI18n();
    const words = t.tools.changes;
    const [all, setAll] = useState(false);
    const shown = all ? changes : changes.slice(0, SHORT_CLOSED);
    return (
        <div className="border-t border-line">
            <h3 className="px-5 pt-4 pb-2 text-ui font-semibold text-ink max-md:px-4">{words.closed}</h3>
            <ul aria-label={words.closedCaption} className="flex flex-col gap-1.5 px-5 pb-4 max-md:px-4">
                {shown.map((change) => (
                    <li
                        key={change.id}
                        className="flex flex-wrap items-center gap-x-2 gap-y-1 text-ui text-ink-2"
                    >
                        {change.closedAt !== null && (
                            <time dateTime={change.closedAt} className="font-mono text-small text-ink-3">
                                {format.timestamp(change.closedAt)}
                            </time>
                        )}
                        {change.closeReason !== null && <Chip>{words.closeReasons[change.closeReason]}</Chip>}
                        <span className="font-mono text-small text-ink [overflow-wrap:anywhere]">
                            {change.path}
                        </span>
                        <span className="text-small text-ink-3">{change.resource}</span>
                    </li>
                ))}
            </ul>
            {changes.length > SHORT_CLOSED && (
                <div className="px-5 pb-4 max-md:px-4">
                    <button
                        type="button"
                        className="text-small font-medium text-ink-2 underline-offset-2 hover:text-ink hover:underline max-md:min-h-11"
                        aria-expanded={all}
                        onClick={() => setAll(!all)}
                    >
                        {all ? words.showFewer : words.showAll(changes.length)}
                    </button>
                </div>
            )}
        </div>
    );
}
