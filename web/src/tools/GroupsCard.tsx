import { Check, ChevronDown, Layers, Lock, Search } from 'lucide-react';
import { useId, useState, type ReactNode } from 'react';

import type { ArchetypeCopy } from '../api/generated/ArchetypeCopy';
import type { ArchetypeGroup } from '../api/generated/ArchetypeGroup';
import type { ArchetypeReport } from '../api/generated/ArchetypeReport';
import type { LeaveReason } from '../api/generated/LeaveReason';
import { hashText } from '../api/tools';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { inputClass } from '../components/Field';
import { table } from '../components/table';
import { Toggle } from '../changes/controls';
import { useI18n } from '../i18n/I18nProvider';
import type { Messages } from '../i18n/en';
import { cn } from '../lib/cn';

/** Rows shown at first, and added by each "Show more": a report can list thousands. */
const PAGE = 100;

type ActionKind = ArchetypeGroup['action']['kind'];
type ReasonKind = LeaveReason['kind'];

const ACTIONS: ActionKind[] = ['fix', 'leave'];
const REASONS: ReasonKind[] = [
    'definitionsDiffer',
    'severalResources',
    'escrowed',
    'interior',
    'mixedSources',
    'contested',
    'manifestDependent',
    'notWritable',
    'excluded',
];

const reasonsOf = (group: ArchetypeGroup): LeaveReason[] =>
    group.action.kind === 'leave' ? group.action.reasons : [];

const matches = (group: ArchetypeGroup, needle: string) =>
    needle === '' ||
    (group.name ?? '').toLowerCase().includes(needle) ||
    hashText(group.hash).toLowerCase().includes(needle) ||
    group.copies.some(
        (copy) => copy.resource.toLowerCase().includes(needle) || copy.path.toLowerCase().includes(needle),
    );

/**
 * Every archetype the report found registered more than once, filtered by what the tool does and why
 * it leaves a group, and searched; `applied` once the run wrote its fixes.
 */
export function GroupsCard({ report, applied }: { report: ArchetypeReport | null; applied: boolean }) {
    const { t } = useI18n();
    const words = t.tools.groups;
    const id = useId();
    const [actions, setActions] = useState<ReadonlySet<ActionKind>>(new Set());
    const [reasons, setReasons] = useState<ReadonlySet<ReasonKind>>(new Set());
    const [query, setQuery] = useState('');
    const [limit, setLimit] = useState(PAGE);
    const groups = report?.groups ?? [];
    const actionCount = (kind: ActionKind) => groups.filter((group) => group.action.kind === kind).length;
    const reasonCount = (kind: ReasonKind) =>
        groups.filter((group) => reasonsOf(group).some((reason) => reason.kind === kind)).length;
    const actionKinds = ACTIONS.filter((kind) => actionCount(kind) > 0);
    const reasonKinds = REASONS.filter((kind) => reasonCount(kind) > 0);
    // A filter picked for one report may name what the next one no longer holds.
    const pickedActions = actionKinds.filter((kind) => actions.has(kind));
    const pickedReasons = reasonKinds.filter((kind) => reasons.has(kind));
    const needle = query.trim().toLowerCase();
    const visible = groups.filter(
        (group) =>
            (pickedActions.length === 0 || pickedActions.includes(group.action.kind)) &&
            (pickedReasons.length === 0 ||
                reasonsOf(group).some((reason) => pickedReasons.includes(reason.kind))) &&
            matches(group, needle),
    );
    const shown = visible.slice(0, limit);
    const actionWord = (kind: ActionKind) =>
        kind === 'leave' ? words.actions.left : applied ? words.actions.fixed : words.actions.toFix;
    const toggle = <T,>(picked: T[], set: (next: ReadonlySet<T>) => void, kind: T, pressed: boolean) => {
        const next = new Set(picked);
        if (pressed) {
            next.add(kind);
        } else {
            next.delete(kind);
        }
        set(next);
        setLimit(PAGE);
    };

    let body: ReactNode;
    if (report === null) {
        body = <p className="px-6 py-6 text-center text-ui text-ink-3">{words.noReport}</p>;
    } else if (groups.length === 0) {
        body = <p className="px-6 py-6 text-center text-ui text-ink-3">{words.none}</p>;
    } else {
        body = (
            <>
                <div className="flex flex-wrap items-center gap-2 border-b border-line px-5 py-3 max-md:px-4">
                    <div className="relative min-w-48 flex-[0_1_18rem] max-md:flex-1">
                        <Search
                            size={14}
                            strokeWidth={1.5}
                            className="pointer-events-none absolute top-1/2 left-2.75 -translate-y-1/2 text-ink-3"
                            aria-hidden="true"
                        />
                        <input
                            type="search"
                            aria-label={words.search}
                            placeholder={words.search}
                            className={cn(inputClass, 'h-8 pl-8.5 text-ui')}
                            value={query}
                            onChange={(event) => {
                                setQuery(event.target.value);
                                setLimit(PAGE);
                            }}
                        />
                    </div>
                    {actionKinds.length > 1 && (
                        <fieldset className="flex flex-wrap gap-1.5">
                            <legend className="sr-only">{words.actionFilters}</legend>
                            {actionKinds.map((kind) => (
                                <Toggle
                                    key={kind}
                                    pressed={pickedActions.includes(kind)}
                                    onChange={(pressed) => toggle(pickedActions, setActions, kind, pressed)}
                                >
                                    {actionWord(kind)}{' '}
                                    <span className="tabular-nums">{actionCount(kind)}</span>
                                </Toggle>
                            ))}
                        </fieldset>
                    )}
                    {reasonKinds.length > 0 && (
                        <fieldset className="flex flex-wrap gap-1.5">
                            <legend className="sr-only">{words.reasonFilters}</legend>
                            {reasonKinds.map((kind) => (
                                <Toggle
                                    key={kind}
                                    pressed={pickedReasons.includes(kind)}
                                    onChange={(pressed) => toggle(pickedReasons, setReasons, kind, pressed)}
                                >
                                    {words.reasons[kind]}{' '}
                                    <span className="tabular-nums">{reasonCount(kind)}</span>
                                </Toggle>
                            ))}
                        </fieldset>
                    )}
                </div>
                {visible.length === 0 ? (
                    <p className="px-6 py-6 text-center text-ui text-ink-3">{words.noMatch}</p>
                ) : (
                    <div className={cn(table.scroll, 'rounded-none')}>
                        <table className={table.table}>
                            <caption className="sr-only">{words.caption}</caption>
                            <thead>
                                <tr>
                                    {[words.archetype, words.copies, words.difference, words.action].map(
                                        (heading) => (
                                            <th key={heading} scope="col" className={table.th}>
                                                {heading}
                                            </th>
                                        ),
                                    )}
                                </tr>
                            </thead>
                            <tbody>
                                {shown.map((group) => (
                                    <GroupRow
                                        key={group.hash}
                                        group={group}
                                        action={actionWord(group.action.kind)}
                                        applied={applied}
                                    />
                                ))}
                            </tbody>
                        </table>
                    </div>
                )}
                <div className="flex flex-wrap items-center gap-2 border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3 max-md:pl-4">
                    <output className="mr-auto">
                        {words.shown(shown.length, visible.length)}
                        {report.groupsTotal > groups.length &&
                            ` · ${words.truncated(groups.length, report.groupsTotal)}`}
                    </output>
                    {visible.length > shown.length && (
                        <Button
                            tone="ghost"
                            size="sm"
                            icon={ChevronDown}
                            onClick={() => setLimit(limit + PAGE)}
                        >
                            {words.showMore}
                        </Button>
                    )}
                </div>
            </>
        );
    }

    return (
        <Card
            id={`${id}-groups`}
            icon={Layers}
            title={words.title}
            meta={report === null ? undefined : words.meta(report.groupsTotal)}
            flush
        >
            {body}
            {report !== null && <ReportFacts report={report} />}
        </Card>
    );
}

function GroupRow({ group, action, applied }: { group: ArchetypeGroup; action: string; applied: boolean }) {
    const { t } = useI18n();
    const words = t.tools.groups;
    const fix = group.action.kind === 'fix' ? group.action : null;
    const escrowed = group.copies.some((copy) => copy.escrowed);
    return (
        <tr className={table.row}>
            <td className={cn(table.td, 'min-w-44 align-top')}>
                <span className={cn(table.mono, 'font-medium')}>{group.name ?? hashText(group.hash)}</span>
                <div className={table.sub}>
                    {group.name !== null && <span className="font-mono">{hashText(group.hash)}</span>}
                    {group.sameFile && <Chip>{words.sameFile}</Chip>}
                    {escrowed && <Chip icon={Lock}>{words.escrowed}</Chip>}
                </div>
            </td>
            <td className={cn(table.td, 'min-w-80 align-top')}>
                <ol className="flex flex-col gap-1.5 py-0.5">
                    {group.copies.map((copy, index) => (
                        <CopyLine
                            // Two copies may share their file, never their place in it.
                            key={`${copy.path} ${copy.source.format === 'ytyp' ? copy.source.index : copy.source.ordinal}`}
                            copy={copy}
                            number={index + 1}
                            role={
                                fix === null
                                    ? null
                                    : fix.keep === index
                                      ? 'kept'
                                      : fix.remove.includes(index)
                                        ? 'removed'
                                        : null
                            }
                        />
                    ))}
                </ol>
            </td>
            <td className={cn(table.td, 'min-w-40 align-top')}>
                <span className="whitespace-nowrap text-ink">{words.differences[group.difference.kind]}</span>
                {group.difference.kind !== 'identical' && group.difference.fields.length > 0 && (
                    <div className={cn(table.sub, 'font-mono [overflow-wrap:anywhere]')}>
                        {group.difference.fields.join(', ')}
                    </div>
                )}
            </td>
            <td className={cn(table.td, 'min-w-64 align-top')}>
                <Chip
                    tone={fix !== null && applied ? 'success' : 'neutral'}
                    icon={fix !== null && applied ? Check : undefined}
                >
                    {action}
                </Chip>
                {fix !== null ? (
                    <p className="mt-1 text-small text-ink-2">
                        {words.keeps(
                            fix.keep + 1,
                            fix.remove.map((index) => index + 1),
                        )}
                    </p>
                ) : (
                    <ul className="mt-1 flex list-disc flex-col gap-0.5 pl-4 text-small text-ink-2">
                        {reasonsOf(group).map((reason) => (
                            <li key={JSON.stringify(reason)} className="[overflow-wrap:anywhere]">
                                {words.reason(reason)}
                            </li>
                        ))}
                    </ul>
                )}
            </td>
        </tr>
    );
}

/** One copy of a group: its number, resource, file and place in the file, and what becomes of it. */
function CopyLine({
    copy,
    number,
    role,
}: {
    copy: ArchetypeCopy;
    number: number;
    role: 'kept' | 'removed' | null;
}) {
    const { t } = useI18n();
    const words = t.tools.groups;
    return (
        <li className="flex min-w-0 gap-2">
            <span className="w-4 flex-none text-right font-mono text-small text-ink-3">{number}</span>
            <span className="flex min-w-0 flex-col">
                <span className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
                    <span className="font-mono text-small font-medium text-ink">{copy.resource}</span>
                    {role !== null && (
                        <Chip tone={role === 'kept' ? 'success' : 'neutral'}>
                            {role === 'kept' ? words.kept : words.removed}
                        </Chip>
                    )}
                </span>
                <span className="font-mono text-meta text-ink-3 [overflow-wrap:anywhere]">
                    {copy.path} · {placeText(t, copy)}
                </span>
            </span>
        </li>
    );
}

const placeText = (t: Messages, copy: ArchetypeCopy) => {
    const words = t.tools.groups;
    const source = copy.source;
    if (source.format === 'modelInfo') {
        return words.item(words.modelInfoKinds[source.kind], source.ordinal, source.line);
    }
    const type = words.archetypeTypes[source.archetype];
    return type === null ? words.entry(source.index) : `${words.entry(source.index)}, ${type}`;
};

/** What the scan read, in counts, and the files and notes that limit what it can tell. */
function ReportFacts({ report }: { report: ArchetypeReport }) {
    const { t, format } = useI18n();
    const words = t.tools.report;
    const { counts } = report;
    return (
        <section
            aria-label={words.label}
            className="flex flex-col gap-2 rounded-b-card border-t border-line px-5 py-4 text-small text-ink-2 max-md:px-4"
        >
            <h3 className="sr-only">{words.label}</h3>
            <ul className="flex flex-col gap-0.5">
                <li>{words.resources(counts.resources, counts.started, counts.client, counts.escrowed)}</li>
                <li>{words.ytyp(counts.ytypLoaded, counts.ytypShadowed, counts.ytypUnreadable)}</li>
                <li>{words.modelInfo(counts.modelInfoLoaded, counts.modelInfoUnreadable)}</li>
                <li>
                    {words.registrations(counts.registrations)},{' '}
                    {words.took(format.milliseconds(report.timings.totalMs))}
                </li>
            </ul>
            {report.shadowedTotal > 0 && (
                <Disclosure
                    title={words.shadowed(report.shadowedTotal)}
                    hint={words.shadowedHint}
                    more={report.shadowedTotal - report.shadowed.length}
                >
                    {report.shadowed.map((file) => (
                        <li key={`${file.resource} ${file.path}`} className="[overflow-wrap:anywhere]">
                            <span className="font-mono text-ink">{file.path}</span>{' '}
                            <span className="text-ink-3">({file.resource})</span>
                            <div className="text-meta text-ink-3">
                                {words.shadowedBy(file.ownerPath, file.ownerResource)}
                            </div>
                        </li>
                    ))}
                </Disclosure>
            )}
            {report.unreadableTotal > 0 && (
                <Disclosure
                    title={words.unreadable(report.unreadableTotal)}
                    hint={words.unreadableHint}
                    more={report.unreadableTotal - report.unreadable.length}
                >
                    {report.unreadable.map((file) => (
                        <li key={`${file.resource} ${file.path}`} className="[overflow-wrap:anywhere]">
                            <span className="font-mono text-ink">{file.path}</span>{' '}
                            <span className="text-ink-3">({file.resource})</span>
                            <div className="text-meta text-ink-3">{words.problem(file.problem)}</div>
                        </li>
                    ))}
                </Disclosure>
            )}
            {report.notesTotal > 0 && (
                <Disclosure
                    title={words.notes(report.notesTotal)}
                    hint={words.notesHint}
                    more={report.notesTotal - report.notes.length}
                >
                    {report.notes.map((note) => {
                        const where = [
                            note.resource,
                            note.file === null
                                ? null
                                : note.line === null
                                  ? note.file
                                  : `${note.file}:${note.line}`,
                            note.detail,
                            note.count > 1 ? words.found(note.count) : null,
                        ].filter((part) => part !== null);
                        return (
                            <li key={JSON.stringify(note)} className="[overflow-wrap:anywhere]">
                                {words.noteKinds[note.kind]}
                                {where.length > 0 && (
                                    <div className="font-mono text-meta text-ink-3">{where.join(' · ')}</div>
                                )}
                            </li>
                        );
                    })}
                </Disclosure>
            )}
        </section>
    );
}

/** A list folded under its title, opened on demand. */
function Disclosure({
    title,
    hint,
    more,
    children,
}: {
    title: string;
    hint: string;
    more: number;
    children: ReactNode;
}) {
    const { t } = useI18n();
    const [open, setOpen] = useState(false);
    return (
        <div className="flex flex-col gap-1.5">
            <button
                type="button"
                aria-expanded={open}
                onClick={() => setOpen(!open)}
                className="inline-flex items-center gap-1.5 self-start text-small font-medium text-ink-2 underline-offset-2 hover:text-ink hover:underline max-md:min-h-11"
            >
                <ChevronDown
                    size={14}
                    strokeWidth={1.5}
                    className={cn('transition-transform', open ? 'rotate-0' : '-rotate-90')}
                    aria-hidden="true"
                />
                {title}
            </button>
            {open && (
                <div className="flex flex-col gap-1.5 pl-5">
                    <p className="text-meta text-ink-3">{hint}</p>
                    <ul className="flex flex-col gap-1.5">{children}</ul>
                    {more > 0 && <p className="text-meta text-ink-3">{t.tools.report.more(more)}</p>}
                </div>
            )}
        </div>
    );
}
