import { useInfiniteQuery, useMutation } from '@tanstack/react-query';
import { ChevronDown, Clock, Download, Filter } from 'lucide-react';
import { useEffect, useId, useRef, useState, type FormEvent, type ReactNode } from 'react';

import { auditQuery, auditSearch, downloadAuditExport, matchesFilter, withLive } from '../api/audit';
import { auditDetails } from '../audit/details';
import type { AuditAction } from '../api/generated/AuditAction';
import type { AuditEntry } from '../api/generated/AuditEntry';
import type { AuditFilter } from '../api/generated/AuditFilter';
import type { Outcome } from '../api/generated/Outcome';
import { Button } from '../components/Button';
import { Chip } from '../components/Chip';
import { inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import type { Messages } from '../i18n/en';
import { cn } from '../lib/cn';
import { useTopic } from '../live/context';

const OUTCOME_TONE = { success: 'success', failure: 'danger', denied: 'warning' } as const;

type Draft = { actor: string; action: AuditAction | ''; outcome: Outcome | ''; from: string; to: string };

const EMPTY: Draft = { actor: '', action: '', outcome: '', from: '', to: '' };

/** The form's local date and time inputs as the API's filter, in UTC. */
const toFilter = (draft: Draft): AuditFilter => ({
    actor: draft.actor.trim() || undefined,
    action: draft.action || undefined,
    outcome: draft.outcome || undefined,
    from: draft.from ? new Date(draft.from).toISOString() : undefined,
    to: draft.to ? new Date(draft.to).toISOString() : undefined,
});

export function AuditPage() {
    const { t } = useI18n();
    const [draft, setDraft] = useState<Draft>(EMPTY);
    const [filter, setFilter] = useState<AuditFilter>({});
    const audit = useInfiniteQuery(auditQuery(filter));
    const loaded = audit.data?.pages.flatMap((page) => page.entries) ?? [];
    // Entries written since the pages were read, newest first, kept while their filter applies.
    const [live, setLive] = useState<{ filter: AuditFilter; entries: AuditEntry[] }>({ filter, entries: [] });
    const liveEntries = live.filter === filter ? live.entries : [];
    const entries = withLive(liveEntries, loaded);
    // The newest entry when the topic last started. Events only bring later ones, and the pages
    // were read at another moment: whatever was written in between comes as neither, so the log
    // is read again whenever the pages, once settled, stop short of it.
    const unchecked = useRef<number | null>(null);
    const catchUp = () => {
        const latest = unchecked.current;
        if (latest === null || !audit.isSuccess || audit.isFetching) {
            return;
        }
        unchecked.current = null;
        if (latest > (loaded[0]?.id ?? 0)) {
            void audit.refetch();
        }
    };
    useEffect(() => {
        catchUp();
    });

    useTopic('audit', {
        event: (entry) => {
            if (matchesFilter(entry, filter)) {
                setLive((current) => ({
                    filter,
                    entries: [entry, ...(current.filter === filter ? current.entries : [])],
                }));
            }
        },
        snapshot: ({ latestId }) => {
            unchecked.current = latestId;
            catchUp();
        },
    });

    const exporting = useMutation({ mutationFn: () => downloadAuditExport(filter) });

    // The same filter keeps its object: the entries that came live are kept while their filter is
    // the one shown, and the query, already up to date, would not read the log again for them.
    const show = (next: AuditFilter) =>
        setFilter((current) => (auditSearch(current) === auditSearch(next) ? current : next));

    const apply = (event: FormEvent) => {
        event.preventDefault();
        show(toFilter(draft));
    };

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.audit.title} <span className="gloss">{t.audit.gloss}</span>
                </h1>
                <div className="flex flex-wrap items-center gap-3 max-md:w-full">
                    <span className="inline-flex items-center gap-1.5 text-ui text-ink-3">
                        <Clock size={14} strokeWidth={1.5} aria-hidden="true" />
                        {t.audit.timeNote}
                    </span>
                    <Button
                        icon={Download}
                        pending={exporting.isPending}
                        onClick={() => exporting.mutate()}
                        className="max-md:flex-1"
                    >
                        {t.audit.export}
                    </Button>
                </div>
            </div>
            {exporting.isError && <Notice tone="danger">{errorMessage(t, exporting.error)}</Notice>}

            <section className="card" aria-labelledby="audit-filters">
                <form className="flex flex-wrap items-end gap-3 p-5 max-md:p-4" onSubmit={apply}>
                    <h2 id="audit-filters" className="sr-only">
                        {t.audit.filters}
                    </h2>
                    <FilterField label={t.audit.actor} className="w-44 max-md:w-full">
                        {(id) => (
                            <input
                                id={id}
                                className={inputClass}
                                value={draft.actor}
                                autoCapitalize="none"
                                spellCheck={false}
                                onChange={(event) => setDraft({ ...draft, actor: event.target.value })}
                            />
                        )}
                    </FilterField>
                    <FilterField label={t.audit.action} className="w-52 max-md:w-full">
                        {(id) => (
                            <Select
                                id={id}
                                value={draft.action}
                                onChange={(action) =>
                                    setDraft({ ...draft, action: action as Draft['action'] })
                                }
                                options={[
                                    ['', t.audit.anyAction],
                                    ...Object.entries(t.audit.actions).toSorted(([, a], [, b]) =>
                                        a.localeCompare(b),
                                    ),
                                ]}
                            />
                        )}
                    </FilterField>
                    <FilterField label={t.audit.outcome} className="w-40 max-md:w-full">
                        {(id) => (
                            <Select
                                id={id}
                                value={draft.outcome}
                                onChange={(outcome) =>
                                    setDraft({ ...draft, outcome: outcome as Draft['outcome'] })
                                }
                                options={[['', t.audit.anyOutcome], ...Object.entries(t.audit.outcomes)]}
                            />
                        )}
                    </FilterField>
                    <FilterField label={t.audit.from} className="w-52 max-md:w-full">
                        {(id) => (
                            <input
                                id={id}
                                type="datetime-local"
                                className={cn(inputClass, 'font-mono text-ui')}
                                value={draft.from}
                                onChange={(event) => setDraft({ ...draft, from: event.target.value })}
                            />
                        )}
                    </FilterField>
                    <FilterField label={t.audit.to} className="w-52 max-md:w-full">
                        {(id) => (
                            <input
                                id={id}
                                type="datetime-local"
                                className={cn(inputClass, 'font-mono text-ui')}
                                value={draft.to}
                                onChange={(event) => setDraft({ ...draft, to: event.target.value })}
                            />
                        )}
                    </FilterField>
                    <div className="flex gap-2 max-md:w-full max-md:[&>button]:flex-1">
                        <Button type="submit" tone="ink" icon={Filter}>
                            {t.audit.apply}
                        </Button>
                        <Button
                            tone="ghost"
                            onClick={() => {
                                setDraft(EMPTY);
                                show({});
                            }}
                        >
                            {t.audit.clear}
                        </Button>
                    </div>
                </form>
            </section>

            <section
                className="card flex flex-col"
                aria-labelledby="audit-entries"
                aria-busy={audit.isFetching}
            >
                <h2 id="audit-entries" className="sr-only">
                    {t.audit.caption}
                </h2>
                {audit.isPending && (
                    <div className="flex flex-col gap-3 p-5">
                        <span className="sr-only">{t.common.loading}</span>
                        <span className="skeleton w-3/4" />
                        <span className="skeleton w-2/3" />
                        <span className="skeleton w-1/2" />
                    </div>
                )}
                {audit.isError && (
                    <div className="p-5 max-md:p-4">
                        <Notice tone="danger">
                            {t.audit.loadFailed} {errorMessage(t, audit.error)}
                        </Notice>
                    </div>
                )}
                {audit.isSuccess && entries.length === 0 && (
                    <p className="px-4 py-6 text-center text-ui text-ink-3">{t.audit.empty}</p>
                )}
                {entries.length > 0 && (
                    <>
                        <div className={table.scroll}>
                            <table className={table.table}>
                                <caption className="sr-only">{t.audit.caption}</caption>
                                <thead>
                                    <tr>
                                        <th scope="col" className={table.th}>
                                            {t.audit.time}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {t.audit.actorColumn}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {t.audit.action}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {t.audit.target}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {t.audit.outcome}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {t.audit.address}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {t.audit.details}
                                        </th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {entries.map((entry) => (
                                        <EntryRow key={entry.id} entry={entry} />
                                    ))}
                                </tbody>
                            </table>
                        </div>
                        <div className="flex flex-wrap items-center gap-2 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3 max-md:pl-4">
                            <output className="mr-auto">
                                {audit.hasNextPage
                                    ? t.audit.shown(entries.length)
                                    : t.audit.allShown(entries.length)}
                            </output>
                            {audit.hasNextPage && (
                                <Button
                                    tone="ghost"
                                    size="sm"
                                    icon={ChevronDown}
                                    pending={audit.isFetchingNextPage}
                                    onClick={() => void audit.fetchNextPage()}
                                >
                                    {t.audit.loadMore}
                                </Button>
                            )}
                        </div>
                    </>
                )}
            </section>
        </div>
    );
}

function EntryRow({ entry }: { entry: AuditEntry }) {
    const { t, format } = useI18n();
    return (
        <tr className={table.row}>
            <td className={cn(table.td, table.mono)}>{format.timestamp(entry.at)}</td>
            <td className={table.td}>
                <span className="font-medium whitespace-nowrap text-ink">{actorName(t, entry)}</span>
                {entry.actorKind !== 'user' && entry.actorName !== null && (
                    <div className={table.sub}>{t.audit.actors[entry.actorKind]}</div>
                )}
            </td>
            <td className={cn(table.td, 'whitespace-nowrap')}>{t.audit.actions[entry.action]}</td>
            <td className={cn(table.td, table.mono)}>
                {entry.target ??
                    (entry.action === 'login_failed' && (
                        <span className="font-sans text-ink-3">{t.audit.unknownUsername}</span>
                    ))}
            </td>
            <td className={table.td}>
                <Chip tone={OUTCOME_TONE[entry.outcome]}>{t.audit.outcomes[entry.outcome]}</Chip>
            </td>
            <td className={cn(table.td, table.mono)}>{entry.ip ?? ''}</td>
            <td className={cn(table.td, 'min-w-60 font-mono text-meta text-ink-3 [overflow-wrap:anywhere]')}>
                {auditDetails(t, format, entry.details, entry.action)}
            </td>
        </tr>
    );
}

const actorName = (t: Messages, entry: AuditEntry) => entry.actorName ?? t.audit.actors[entry.actorKind];

function FilterField({
    label,
    className,
    children,
}: {
    label: string;
    className?: string;
    children: (id: string) => ReactNode;
}) {
    const id = useId();
    return (
        <div className={cn('flex min-w-0 flex-col gap-1.5', className)}>
            <label htmlFor={id} className="text-ui font-medium text-ink">
                {label}
            </label>
            {children(id)}
        </div>
    );
}

function Select({
    id,
    value,
    options,
    onChange,
}: {
    id: string;
    value: string;
    options: [string, string][];
    onChange: (value: string) => void;
}) {
    return (
        <div className="relative">
            <select
                id={id}
                className={cn(inputClass, 'appearance-none pr-9')}
                value={value}
                onChange={(event) => onChange(event.target.value)}
            >
                {options.map(([option, label]) => (
                    <option key={option} value={option}>
                        {label}
                    </option>
                ))}
            </select>
            <ChevronDown
                size={16}
                strokeWidth={1.5}
                className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2 text-ink-3"
                aria-hidden="true"
            />
        </div>
    );
}
