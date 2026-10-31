import { keepPreviousData, useInfiniteQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowDown, ChevronDown, ChevronsUpDown, Contact, Search } from 'lucide-react';
import { useId, useState, type ReactNode } from 'react';

import { ApiError } from '../api/client';
import { Permission } from '../api/generated/Permission';
import type { PlayerSort } from '../api/generated/PlayerSort';
import type { PlayerSummary } from '../api/generated/PlayerSummary';
import { playersQuery, shortLicence, totalOf } from '../api/players';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { CopyValue } from '../components/CopyValue';
import { TextField, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { errorMessage } from '../i18n/errors';
import { zoneDate } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { OnlineState, PlayerName, useVisit } from './words';

const SORTS: PlayerSort[] = ['playTime', 'recent', 'sessions', 'firstSeen', 'lastSeen'];

/** Refusals of what was typed, said under the search field rather than in place of the list. */
const SEARCH_REFUSALS = new Set(['forbidden', 'address_search', 'invalid_search']);

/** Every profile, searched and sorted, a page at a time. */
export function PlayersCard() {
    const { t } = useI18n();
    const words = t.players.list;
    const me = useMe();
    const sortId = useId();
    const [draft, setDraft] = useState('');
    const [search, setSearch] = useState('');
    const [sort, setSort] = useState<PlayerSort>('playTime');
    const list = useInfiniteQuery({ ...playersQuery(search, sort), placeholderData: keepPreviousData });
    const players = list.data?.pages.flatMap((page) => page.players) ?? [];
    // Visits read in the schedule's zone, which the list names itself; there are none to show before it.
    const zone = list.data?.pages[0]?.zone ?? 'UTC';
    const refused =
        list.error instanceof ApiError && SEARCH_REFUSALS.has(list.error.code)
            ? errorMessage(t, list.error)
            : null;
    const identifiers = allows(me, Permission.PlayerIdentifiersView);

    return (
        <Card id="players-list" icon={Contact} title={words.title} flush>
            <form
                className="flex flex-wrap items-start gap-3 border-b border-line p-5 max-md:p-4"
                onSubmit={(event) => {
                    event.preventDefault();
                    setSearch(draft.trim());
                }}
            >
                <TextField
                    label={words.search}
                    hint={identifiers ? words.searchHintIdentifiers : words.searchHint}
                    error={refused ?? undefined}
                    type="search"
                    value={draft}
                    spellCheck={false}
                    autoCapitalize="none"
                    autoComplete="off"
                    className="min-w-0 flex-1 basis-72"
                    inputClassName="pl-8.5"
                    after={
                        <Search
                            size={16}
                            strokeWidth={1.5}
                            className="pointer-events-none absolute top-1/2 left-2.75 -translate-y-1/2 text-ink-3"
                            aria-hidden="true"
                        />
                    }
                    onChange={(event) => setDraft(event.target.value)}
                />
                <div className="flex gap-2 pt-6.5 max-md:w-full max-md:pt-0 max-md:[&>button]:flex-1">
                    <Button type="submit" icon={Search}>
                        {words.submit}
                    </Button>
                    {search !== '' && (
                        <Button
                            tone="ghost"
                            onClick={() => {
                                setDraft('');
                                setSearch('');
                            }}
                        >
                            {words.clear}
                        </Button>
                    )}
                </div>
                <div className="flex w-full flex-col gap-1.5 md:hidden">
                    <label htmlFor={sortId} className="text-ui font-medium text-ink">
                        {words.sort}
                    </label>
                    <div className="relative">
                        <select
                            id={sortId}
                            className={cn(inputClass, 'appearance-none pr-9')}
                            value={sort}
                            onChange={(event) => setSort(event.target.value as PlayerSort)}
                        >
                            {SORTS.map((each) => (
                                <option key={each} value={each}>
                                    {words.sorts[each]}
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
                </div>
            </form>
            <div aria-busy={list.isFetching} className={cn(list.isPlaceholderData && 'opacity-50')}>
                {list.isPending && (
                    <div className="flex flex-col gap-3 p-5">
                        <span className="sr-only">{words.loading}</span>
                        <span className="skeleton w-3/4" />
                        <span className="skeleton w-2/3" />
                        <span className="skeleton w-1/2" />
                    </div>
                )}
                {list.isError && refused === null && (
                    <div className="p-5 max-md:p-4">
                        <Notice tone="danger">
                            {words.failed} {errorMessage(t, list.error)}
                        </Notice>
                    </div>
                )}
                {list.isSuccess && players.length === 0 && (
                    <p className="px-6 py-4 text-center text-ui text-ink-3">
                        {search === '' ? words.empty : words.noMatch}
                    </p>
                )}
                {players.length > 0 && refused === null && (
                    <>
                        <div className={cn(table.scroll, 'rounded-none max-md:hidden')}>
                            <table className={table.table}>
                                <caption className="sr-only">{words.caption(words.sorts[sort])}</caption>
                                <thead>
                                    <tr>
                                        <th scope="col" className={table.th}>
                                            {words.name}
                                        </th>
                                        <th scope="col" className={table.th}>
                                            {words.licence}
                                        </th>
                                        {SORTS.map((each) => (
                                            <SortHeader
                                                key={each}
                                                active={sort === each}
                                                numeric={each !== 'firstSeen' && each !== 'lastSeen'}
                                                onSort={() => setSort(each)}
                                            >
                                                {words.sorts[each]}
                                            </SortHeader>
                                        ))}
                                        <th scope="col" className={table.th}>
                                            {words.state}
                                        </th>
                                    </tr>
                                </thead>
                                <tbody>
                                    {players.map((player) => (
                                        <PlayerRow key={player.id} player={player} zone={zone} />
                                    ))}
                                </tbody>
                            </table>
                        </div>
                        <ul className="flex flex-col md:hidden">
                            {players.map((player) => (
                                <PlayerItem key={player.id} player={player} zone={zone} />
                            ))}
                        </ul>
                    </>
                )}
            </div>
            <div className="flex flex-wrap items-center gap-2 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3 max-md:pl-4">
                <span className="mr-auto">{t.players.timeConnected}</span>
                {players.length > 0 && refused === null && (
                    <output>
                        {list.hasNextPage ? words.shown(players.length) : words.allShown(players.length)}
                    </output>
                )}
                {list.hasNextPage && refused === null && (
                    <Button
                        tone="ghost"
                        size="sm"
                        icon={ChevronDown}
                        pending={list.isFetchingNextPage}
                        onClick={() => void list.fetchNextPage()}
                    >
                        {words.loadMore}
                    </Button>
                )}
            </div>
        </Card>
    );
}

/** A column the list sorts by, largest or latest first: the one in use says so. */
function SortHeader({
    active,
    numeric,
    onSort,
    children,
}: {
    active: boolean;
    numeric: boolean;
    onSort: () => void;
    children: ReactNode;
}) {
    return (
        <th
            scope="col"
            aria-sort={active ? 'descending' : undefined}
            className={cn(table.th, numeric && 'text-right')}
        >
            <button
                type="button"
                className={cn(
                    '-mx-1 inline-flex items-center gap-1 rounded-xs px-1 hover:text-ink',
                    active && 'text-ink',
                )}
                onClick={onSort}
            >
                {children}
                {active ? (
                    <ArrowDown size={12} strokeWidth={2} aria-hidden="true" />
                ) : (
                    <ChevronsUpDown size={12} strokeWidth={2} className="opacity-55" aria-hidden="true" />
                )}
            </button>
        </th>
    );
}

function Name({ player }: { player: PlayerSummary }) {
    return (
        <Link
            to="/players/$id"
            params={{ id: String(player.id) }}
            className="font-medium text-ink underline-offset-2 [overflow-wrap:anywhere] hover:underline"
        >
            <PlayerName name={player.name} />
        </Link>
    );
}

function PlayerRow({ player, zone }: { player: PlayerSummary; zone: string }) {
    const { t, format } = useI18n();
    const visit = useVisit(zone);
    const number = cn(table.td, 'text-right whitespace-nowrap');
    return (
        <tr className={table.row}>
            <td className={cn(table.td, 'min-w-40')}>
                <Name player={player} />
            </td>
            <td className={table.td}>
                <CopyValue
                    value={player.license}
                    shown={shortLicence(player.license)}
                    label={t.players.copyLicence(player.license)}
                />
            </td>
            <td className={cn(number, 'font-semibold text-ink')}>
                {format.playTime(totalOf(player))}
                {player.importedMs > 0 && (
                    <div className={cn(table.sub, 'justify-end font-normal')}>
                        {t.players.list.imported(format.playTime(player.importedMs))}
                    </div>
                )}
            </td>
            <td className={number}>{format.playTime(player.recentMs + player.inProgressMs)}</td>
            <td className={number}>{format.number(player.sessions)}</td>
            <td className={cn(table.td, table.mono)}>{visit(player.firstSeen)}</td>
            <td className={cn(table.td, table.mono)}>{visit(player.lastSeen)}</td>
            <td className={table.td}>
                <OnlineState online={player.online} />
            </td>
        </tr>
    );
}

function PlayerItem({ player, zone }: { player: PlayerSummary; zone: string }) {
    const { t, format } = useI18n();
    const sorts = t.players.list.sorts;
    return (
        <li className="flex flex-col gap-1.5 border-b border-line px-4 py-3 last:border-b-0">
            <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                <Name player={player} />
                <OnlineState online={player.online} />
            </div>
            <CopyValue
                value={player.license}
                shown={shortLicence(player.license)}
                label={t.players.copyLicence(player.license)}
            />
            <dl className="grid grid-cols-2 gap-x-4 gap-y-1 text-small">
                <Figure term={sorts.playTime}>{format.playTime(totalOf(player))}</Figure>
                <Figure term={sorts.recent}>{format.playTime(player.recentMs + player.inProgressMs)}</Figure>
                <Figure term={sorts.sessions}>{format.number(player.sessions)}</Figure>
                <Figure term={sorts.lastSeen}>
                    <span className="font-mono">{zoneDate(player.lastSeen, zone)}</span>
                </Figure>
            </dl>
        </li>
    );
}

function Figure({ term, children }: { term: string; children: ReactNode }) {
    return (
        <div className="flex min-w-0 flex-col">
            <dt className="text-meta text-ink-3">{term}</dt>
            <dd className="font-medium text-ink">{children}</dd>
        </div>
    );
}
