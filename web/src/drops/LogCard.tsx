import { keepPreviousData, useInfiniteQuery, useMutation } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ChevronDown, Download, Filter, ScrollText, X } from 'lucide-react';
import { useId, useRef, useState, type ReactNode, type Ref } from 'react';

import { ApiError } from '../api/client';
import { downloadLog, logQuery } from '../api/drops';
import type { DropCategory } from '../api/generated/DropCategory';
import type { DropKind } from '../api/generated/DropKind';
import type { DropLogRow } from '../api/generated/DropLogRow';
import { Permission } from '../api/generated/Permission';
import type { SignatureTable } from '../api/generated/SignatureTable';
import { shortLicence } from '../api/players';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { CopyValue } from '../components/CopyValue';
import { FieldError, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { Toggletip } from '../components/Toggletip';
import { errorMessage } from '../i18n/errors';
import { formatClock, zoneDate } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { DROP_ICONS, PlayerName } from '../players/words';
import {
    CATEGORIES,
    DEFAULT_RANGE,
    KINDS,
    filtered,
    logFilters,
    logSearch,
    type DropsSearch,
    type LogFilters,
} from './search';

/** Refusals of what was asked for one player, said where it was asked rather than in place of the log. */
const SEARCH_REFUSALS = new Set(['forbidden', 'address_search', 'invalid_search']);

/** The log's filters, without the page's range and chart. */
type Filters = Omit<DropsSearch, 'range' | 'chart'>;

type Draft = {
    category: DropCategory | '';
    kind: DropKind | '';
    resource: string;
    signature: string;
    player: string;
    run: string;
};

const draftOf = (search: Filters): Draft => ({
    category: search.category ?? '',
    kind: search.kind ?? '',
    resource: search.resource ?? '',
    signature: search.signature === undefined ? '' : String(search.signature),
    player: search.player ?? '',
    run: search.run === undefined ? '' : String(search.run),
});

/** A run as typed: empty, or a whole number; anything else is refused under its field. */
const runOf = (draft: Draft) => draft.run.trim();
const runValid = (draft: Draft) => runOf(draft) === '' || /^\d+$/.test(runOf(draft));

/**
 * The form as the address takes it: what is empty is left out. The API reads one player at a time,
 * a group before a search before a profile: a player typed replaces the profile or the group kept.
 */
const searchOf = (draft: Draft, search: Filters): Filters => {
    const player = draft.player.trim() || undefined;
    return {
        category: draft.category || undefined,
        kind: draft.kind || undefined,
        resource: draft.resource.trim() || undefined,
        signature: draft.signature === '' ? undefined : Number(draft.signature),
        player,
        run: runOf(draft) === '' ? undefined : Number(runOf(draft)),
        profile: player === undefined ? search.profile : undefined,
        group: player === undefined ? search.group : undefined,
    };
};

/** The filters as one key, whatever the range: the form starts again from them when they change elsewhere. */
const filtersKey = (search: DropsSearch) => logSearch({ ...logFilters(search), range: DEFAULT_RANGE });

/**
 * Every drop under the filters, newest first, a page at a time: who, what, how long they played and
 * in which run. Names and texts are written as plain text; the filters live in the address.
 */
export function LogCard({
    search,
    signatures,
    zone,
    titleRef,
    onFilters,
}: {
    search: DropsSearch;
    /** The range's signatures, which the signature filter offers. */
    signatures: SignatureTable | undefined;
    zone: string;
    titleRef: Ref<HTMLHeadingElement>;
    onFilters: (next: Filters) => void;
}) {
    const { t } = useI18n();
    const words = t.drops.log;
    const filters = logFilters(search);
    const log = useInfiniteQuery({ ...logQuery(filters), placeholderData: keepPreviousData });
    const rows = log.data?.pages.flatMap((page) => page.rows) ?? [];
    const refusal =
        log.error instanceof ApiError && SEARCH_REFUSALS.has(log.error.code)
            ? errorMessage(t, log.error)
            : null;
    // Said once more pages were loaded, never as the figures are read again.
    const [announced, setAnnounced] = useState<{ key: string; text: string } | null>(null);
    const exporting = useMutation({ mutationFn: (current: LogFilters) => downloadLog(current) });
    const stale = log.isPlaceholderData;
    const shown = (count: number, more: boolean) => (more ? words.shown(count) : words.allShown(count));
    return (
        <Card
            id="drops-log"
            icon={ScrollText}
            title={words.title}
            titleRef={titleRef}
            meta={words.meta(t.drops.over[filters.range])}
            aside={
                <Button
                    size="sm"
                    icon={Download}
                    pending={exporting.isPending}
                    onClick={() => exporting.mutate(filters)}
                >
                    {words.export}
                </Button>
            }
            flush
        >
            <LogFilterForm
                search={search}
                signatures={signatures}
                refusedPlayer={search.player === undefined ? null : refusal}
                refusedKept={search.player === undefined ? refusal : null}
                onFilters={onFilters}
            />
            {exporting.isError && (
                <div className="border-b border-line px-5 py-3 max-md:px-4">
                    <Notice tone="danger">
                        {t.drops.exportFailed} {errorMessage(t, exporting.error)}
                    </Notice>
                </div>
            )}
            <div aria-busy={stale} className={cn('transition-opacity', stale && 'opacity-50')}>
                {log.isPending && (
                    <div className="flex flex-col gap-3 p-5">
                        <span className="sr-only">{words.reading}</span>
                        <span className="skeleton w-3/4" />
                        <span className="skeleton w-2/3" />
                        <span className="skeleton w-1/2" />
                    </div>
                )}
                {log.isError && refusal === null && (
                    <div className="p-5 max-md:p-4">
                        <Notice tone="danger" live={false}>
                            {words.failed} {errorMessage(t, log.error)}
                        </Notice>
                    </div>
                )}
                {log.isSuccess && rows.length === 0 && (
                    <p className="px-6 py-4 text-center text-ui text-ink-3">
                        {filtered(search) ? words.noMatch : words.empty}
                    </p>
                )}
                {rows.length > 0 && refusal === null && (
                    <>
                        <div className={cn(table.scroll, 'rounded-none max-md:hidden')}>
                            <table className={table.table}>
                                <caption className="sr-only">{words.caption}</caption>
                                <thead>
                                    <tr>
                                        <th scope="col" className={table.th}>
                                            {words.time}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {words.playerColumn}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {words.categoryColumn}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {words.detail}
                                        </th>
                                        <th scope="col" className={cn(table.th, 'text-right')}>
                                            {words.session}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {words.phase}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {words.runColumn}
                                        </th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {rows.map((row) => (
                                        <LogRow key={row.id} row={row} zone={zone} onFilter={onFilters} />
                                    ))}
                                </tbody>
                            </table>
                        </div>
                        <ul className="flex flex-col md:hidden">
                            {rows.map((row) => (
                                <LogItem key={row.id} row={row} zone={zone} onFilter={onFilters} />
                            ))}
                        </ul>
                    </>
                )}
            </div>
            {rows.length > 0 && refusal === null && (
                <div className="flex flex-wrap items-center gap-2 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3 max-md:pl-4">
                    <span className="mr-auto">{shown(rows.length, log.hasNextPage)}</span>
                    <output className="sr-only">
                        {announced?.key === logSearch(filters) ? announced.text : ''}
                    </output>
                    {log.hasNextPage && (
                        <Button
                            tone="ghost"
                            size="sm"
                            icon={ChevronDown}
                            pending={log.isFetchingNextPage}
                            onClick={() => {
                                const key = logSearch(filters);
                                void log.fetchNextPage().then((result) => {
                                    const count = result.data?.pages.flatMap((page) => page.rows).length ?? 0;
                                    setAnnounced({ key, text: shown(count, result.hasNextPage) });
                                });
                            }}
                        >
                            {words.loadMore}
                        </Button>
                    )}
                </div>
            )}
        </Card>
    );
}

function LogFilterForm({
    search,
    signatures,
    refusedPlayer,
    refusedKept,
    onFilters,
}: {
    search: DropsSearch;
    signatures: SignatureTable | undefined;
    /** Why the API refused the player typed. */
    refusedPlayer: string | null;
    /** Why it refused the profile or group the address keeps. */
    refusedKept: string | null;
    onFilters: (next: Filters) => void;
}) {
    const { t } = useI18n();
    const words = t.drops.log;
    const me = useMe();
    const playerId = useId();
    const runId = useId();
    const keptId = useId();
    const apply = useRef<HTMLButtonElement>(null);
    const [draft, setDraft] = useState(() => draftOf(search));
    const [runChecked, setRunChecked] = useState(false);
    // The filters changed elsewhere, a card's link or the address: the form starts again from them.
    // The range, the chart or the breakdown leave a draft as it is.
    const applied = filtersKey(search);
    const [from, setFrom] = useState(applied);
    if (from !== applied) {
        setFrom(applied);
        setDraft(draftOf(search));
        setRunChecked(false);
    }
    const runError = runChecked && !runValid(draft);
    const kinds = draft.category === '' ? [] : KINDS[draft.category];
    const listed = signatures?.signatures.filter((entry) => entry.signature !== null) ?? [];
    const signatureOptions: [string, string][] = [
        ['', words.anySignature],
        ...listed.map((entry): [string, string] => [String(entry.id), entry.signature ?? '']),
    ];
    // A signature the address names that this range does not list still shows, by its number.
    if (draft.signature !== '' && !listed.some((entry) => String(entry.id) === draft.signature)) {
        signatureOptions.push([draft.signature, words.signatureId(Number(draft.signature))]);
    }
    const kept = [
        search.profile === undefined
            ? null
            : { name: words.profile(search.profile), drop: 'profile' as const },
        search.group === undefined ? null : { name: words.group(search.group), drop: 'group' as const },
    ].filter((each) => each !== null);
    return (
        <form
            className="flex flex-col gap-3 border-b border-line p-5 max-md:p-4"
            aria-label={words.filters}
            noValidate
            onSubmit={(event) => {
                event.preventDefault();
                setRunChecked(true);
                if (runValid(draft)) {
                    onFilters(searchOf(draft, search));
                }
            }}
        >
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3 3xl:grid-cols-6">
                <FilterField label={words.category}>
                    {(id) => (
                        <Select
                            id={id}
                            value={draft.category}
                            onChange={(category) =>
                                setDraft({
                                    ...draft,
                                    category: category as Draft['category'],
                                    // A kind belongs to its category: another category leaves it out.
                                    kind:
                                        category !== '' &&
                                        KINDS[category as DropCategory].includes(draft.kind as DropKind)
                                            ? draft.kind
                                            : '',
                                })
                            }
                            options={[
                                ['', words.anyCategory],
                                ...CATEGORIES.map((category): [string, string] => [
                                    category,
                                    t.drops.categories[category],
                                ]),
                            ]}
                        />
                    )}
                </FilterField>
                <FilterField label={words.kind}>
                    {(id) => (
                        <Select
                            id={id}
                            value={draft.kind}
                            disabled={kinds.length === 0}
                            onChange={(kind) => setDraft({ ...draft, kind: kind as Draft['kind'] })}
                            options={[
                                ['', words.anyKind],
                                ...kinds.map((kind): [string, string] => [kind, t.drops.kinds[kind]]),
                            ]}
                        />
                    )}
                </FilterField>
                <FilterField label={words.resource}>
                    {(id) => (
                        <input
                            id={id}
                            className={cn(inputClass, 'font-mono text-ui')}
                            value={draft.resource}
                            spellCheck={false}
                            autoCapitalize="none"
                            autoComplete="off"
                            onChange={(event) => setDraft({ ...draft, resource: event.target.value })}
                        />
                    )}
                </FilterField>
                <FilterField label={words.signature}>
                    {(id) => (
                        <Select
                            id={id}
                            value={draft.signature}
                            onChange={(signature) => setDraft({ ...draft, signature })}
                            options={signatureOptions}
                        />
                    )}
                </FilterField>
                <div className="flex min-w-0 flex-col gap-1.5">
                    <label htmlFor={playerId} className="text-ui font-medium text-ink">
                        {words.player}
                    </label>
                    <input
                        id={playerId}
                        type="search"
                        className={inputClass}
                        value={draft.player}
                        spellCheck={false}
                        autoCapitalize="none"
                        autoComplete="off"
                        aria-invalid={refusedPlayer !== null || undefined}
                        aria-describedby={`${playerId}-note`}
                        onChange={(event) => setDraft({ ...draft, player: event.target.value })}
                    />
                </div>
                <div className="flex min-w-0 flex-col gap-1.5">
                    <label htmlFor={runId} className="text-ui font-medium text-ink">
                        {words.run}
                    </label>
                    <input
                        id={runId}
                        inputMode="numeric"
                        className={cn(inputClass, 'font-mono text-ui')}
                        value={draft.run}
                        autoComplete="off"
                        aria-invalid={runError || undefined}
                        aria-describedby={runError ? `${runId}-error` : undefined}
                        onChange={(event) => setDraft({ ...draft, run: event.target.value })}
                    />
                </div>
            </div>
            {refusedPlayer !== null ? (
                <FieldError id={`${playerId}-note`}>{refusedPlayer}</FieldError>
            ) : (
                <p id={`${playerId}-note`} className="text-small text-ink-3">
                    {allows(me, Permission.PlayerIdentifiersView)
                        ? words.playerHintIdentifiers
                        : words.playerHint}
                </p>
            )}
            {runError && <FieldError id={`${runId}-error`}>{words.runInvalid}</FieldError>}
            {refusedKept !== null && <FieldError id={keptId}>{refusedKept}</FieldError>}
            <div className="flex flex-wrap items-center gap-2 max-md:[&>button]:flex-1">
                {kept.map((filter) => (
                    <Button
                        key={filter.drop}
                        size="sm"
                        tone="secondary"
                        icon={X}
                        aria-label={words.removeFilter(filter.name)}
                        aria-describedby={refusedKept === null ? undefined : keptId}
                        onClick={() => {
                            // The chip goes: the keyboard goes on to the form's own button.
                            onFilters({ ...searchOf(draft, search), [filter.drop]: undefined });
                            apply.current?.focus();
                        }}
                    >
                        {filter.name}
                    </Button>
                ))}
                <Button ref={apply} type="submit" tone="ink" icon={Filter} className="md:ml-auto">
                    {words.apply}
                </Button>
                <Button
                    tone="ghost"
                    onClick={() => {
                        setDraft(draftOf({}));
                        setRunChecked(false);
                        onFilters({});
                    }}
                >
                    {words.clear}
                </Button>
            </div>
        </form>
    );
}

function FilterField({ label, children }: { label: string; children: (id: string) => ReactNode }) {
    const id = useId();
    return (
        <div className="flex min-w-0 flex-col gap-1.5">
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
    disabled,
    onChange,
}: {
    id: string;
    value: string;
    options: [string, string][];
    disabled?: boolean;
    onChange: (value: string) => void;
}) {
    return (
        <div className="relative">
            <select
                id={id}
                className={cn(inputClass, 'appearance-none truncate pr-9')}
                value={value}
                disabled={disabled}
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

/** A drop's moment as logs write it, in the server's zone and to the second: `2026-10-28 14:02:07`. */
const useStamp = (zone: string) => {
    const { locale } = useI18n();
    return (instant: string) =>
        `${zoneDate(instant, zone)} ${formatClock(locale, instant, zone, { seconds: true })}`;
};

/** Who dropped: their name to their profile and their licence to copy; or why nobody is named. */
function Who({ row }: { row: DropLogRow }) {
    const { t } = useI18n();
    const words = t.drops.log;
    if (row.player === null) {
        return row.synthesized ? (
            <span className="text-ink-3">{words.noPlayer}</span>
        ) : (
            <Toggletip
                className="self-start rounded-xs text-left text-ink-2 underline decoration-dotted underline-offset-2"
                tip={words.unconfirmedHint}
            >
                {words.unconfirmed}
            </Toggletip>
        );
    }
    return (
        <div className="flex min-w-0 flex-col gap-0.5">
            <Link
                to="/players/$id"
                params={{ id: String(row.player.id) }}
                className="font-medium text-ink underline-offset-2 [overflow-wrap:anywhere] hover:underline"
            >
                <PlayerName name={row.player.name ?? ''} />
            </Link>
            <CopyValue
                value={row.player.license}
                shown={shortLicence(row.player.license)}
                label={t.players.copyLicence(row.player.license)}
            />
        </div>
    );
}

function Category({ row }: { row: DropLogRow }) {
    const { t } = useI18n();
    const Icon = DROP_ICONS[row.category];
    return (
        <span className="inline-flex items-center gap-1.5 whitespace-nowrap text-ink">
            <Icon size={14} strokeWidth={1.5} className="text-ink-3" aria-hidden="true" />
            {t.players.profile.drops.categories[row.category]}
        </span>
    );
}

/** Narrows the log to one thing a row shows: its signature, its run. */
type OnFilter = (next: Filters) => void;

/**
 * What the drop says: its signature, which narrows the log to it, the resource and its reason, its
 * kind; all as plain text.
 */
function Detail({ row, onFilter }: { row: DropLogRow; onFilter: OnFilter }) {
    const { t } = useI18n();
    const words = t.drops.log;
    const notes = [
        row.synthesized ? words.closedByObserver : null,
        row.incident ? words.duringIncident : null,
    ].filter((note) => note !== null);
    return (
        <div className="flex min-w-0 flex-col gap-0.5">
            {row.signature !== null &&
                (row.signatureId === null ? (
                    <span className={cn(table.mono, 'whitespace-normal [overflow-wrap:anywhere]')}>
                        {row.signature}
                    </span>
                ) : (
                    <FilterButton
                        label={words.filterSignature(row.signature)}
                        onClick={() => onFilter({ signature: row.signatureId ?? undefined })}
                    >
                        <span className="whitespace-normal text-ink [overflow-wrap:anywhere]">
                            {row.signature}
                        </span>
                    </FilterButton>
                ))}
            {row.label !== null && <span className="font-mono text-meta text-ink-3">{row.label}</span>}
            {row.resource !== null && (
                <span>
                    {words.byResource(
                        <code key="resource" className="code-inline">
                            {row.resource}
                        </code>,
                    )}
                </span>
            )}
            {row.reason !== null && (
                <span className="whitespace-pre-wrap text-ink [overflow-wrap:anywhere]">{row.reason}</span>
            )}
            {row.kind !== null && row.resource === null && <span>{t.drops.kinds[row.kind]}</span>}
            {notes.length > 0 && <span className="text-meta text-ink-3">{notes.join(' · ')}</span>}
        </div>
    );
}

const phase = (t: ReturnType<typeof useI18n>['t'], loading: boolean | null) =>
    loading === null ? '' : loading ? t.drops.log.loadingPhase : t.drops.log.inGame;

/** A value of a row in mono that narrows the log to it: said in its name, and in its title. */
function FilterButton({
    label,
    onClick,
    children,
}: {
    label: string;
    onClick: () => void;
    children: ReactNode;
}) {
    return (
        <button
            type="button"
            title={label}
            aria-label={label}
            className="self-start rounded-xs text-left font-mono text-small text-ink-2 underline-offset-2 [font-variant-ligatures:none] hover:text-ink hover:underline max-md:min-h-11"
            onClick={onClick}
        >
            {children}
        </button>
    );
}

function RunButton({ run, onFilter }: { run: number; onFilter: OnFilter }) {
    const { t } = useI18n();
    return (
        <FilterButton label={t.drops.log.filterRun(run)} onClick={() => onFilter({ run })}>
            <span className="whitespace-nowrap">{t.drops.log.runValue(run)}</span>
        </FilterButton>
    );
}

function LogRow({ row, zone, onFilter }: { row: DropLogRow; zone: string; onFilter: OnFilter }) {
    const { t, format } = useI18n();
    const stamp = useStamp(zone);
    return (
        <tr className={table.row}>
            <td className={cn(table.td, table.mono, 'align-top')}>
                <time dateTime={row.at}>{stamp(row.at)}</time>
            </td>
            <td className={cn(table.td, 'min-w-36 align-top')}>
                <Who row={row} />
            </td>
            <td className={cn(table.td, 'align-top')}>
                <Category row={row} />
            </td>
            <td className={cn(table.td, 'min-w-48 align-top')}>
                <Detail row={row} onFilter={onFilter} />
            </td>
            <td className={cn(table.td, 'text-right align-top whitespace-nowrap')}>
                {row.lengthMs === null ? '' : format.playTime(row.lengthMs)}
            </td>
            <td className={cn(table.td, 'align-top whitespace-nowrap')}>{phase(t, row.loading)}</td>
            <td className={cn(table.td, 'align-top')}>
                <RunButton run={row.run} onFilter={onFilter} />
            </td>
        </tr>
    );
}

function LogItem({ row, zone, onFilter }: { row: DropLogRow; zone: string; onFilter: OnFilter }) {
    const { t, format } = useI18n();
    const words = t.drops.log;
    const stamp = useStamp(zone);
    // Each figure's line as tall as the run's button, so that the three stand level.
    const figure = 'flex min-h-11 items-center text-ink';
    return (
        <li className="flex flex-col gap-1.5 border-b border-line px-4 py-3 last:border-b-0">
            <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                <Category row={row} />
                <time dateTime={row.at} className="font-mono text-small text-ink-3">
                    {stamp(row.at)}
                </time>
            </div>
            <Who row={row} />
            <div className="text-small text-ink-2">
                <Detail row={row} onFilter={onFilter} />
            </div>
            <dl className="grid grid-cols-3 gap-x-4 text-small">
                <div className="flex min-w-0 flex-col">
                    <dt className="text-meta text-ink-3">{words.session}</dt>
                    <dd className={figure}>{row.lengthMs === null ? '' : format.playTime(row.lengthMs)}</dd>
                </div>
                <div className="flex min-w-0 flex-col">
                    <dt className="text-meta text-ink-3">{words.phase}</dt>
                    <dd className={figure}>{phase(t, row.loading)}</dd>
                </div>
                <div className="flex min-w-0 flex-col">
                    <dt className="text-meta text-ink-3">{words.runColumn}</dt>
                    <dd className={figure}>
                        <RunButton run={row.run} onFilter={onFilter} />
                    </dd>
                </div>
            </dl>
        </li>
    );
}
