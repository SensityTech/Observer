import { keepPreviousData, useInfiniteQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { Ban as BanIcon, ChevronDown, Plus, Search, Undo2 } from 'lucide-react';
import { useState } from 'react';

import { bansQuery } from '../api/bans';
import { ApiError } from '../api/client';
import type { Ban } from '../api/generated/Ban';
import type { BanFilter } from '../api/generated/BanFilter';
import { Permission } from '../api/generated/Permission';
import { shortLicence } from '../api/players';
import { allows, useMe } from '../auth/me';
import { BanDialog, LiftDialog } from '../bans/dialogs';
import { BanEnds } from '../bans/words';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { CopyValue } from '../components/CopyValue';
import { TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { table } from '../components/table';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useVisit, ZoneNote } from '../players/words';
import { useSchedule } from '../schedule/context';

const FILTERS: BanFilter[] = ['active', 'ended', 'lifted', 'all'];
/** Refusals of what was typed, said under the search field rather than in place of the list. */
const SEARCH_REFUSALS = new Set(['forbidden', 'address_search', 'invalid_search']);

type Done = { tone: 'success'; message: string };

export function BansPage() {
    const { t } = useI18n();
    const words = t.bans;
    const me = useMe();
    const [filter, setFilter] = useState<BanFilter>('active');
    const [draft, setDraft] = useState('');
    const [search, setSearch] = useState('');
    const [banning, setBanning] = useState(false);
    const [lifting, setLifting] = useState<Ban | null>(null);
    const [done, setDone] = useState<Done | null>(null);
    const list = useInfiniteQuery({ ...bansQuery(filter, search), placeholderData: keepPreviousData });
    const scheduled = useSchedule().zone;
    const bans = list.data?.pages.flatMap((page) => page.bans) ?? [];
    const first = list.data?.pages[0];
    // Times read in the schedule's zone, which the list names itself.
    const zone = first?.zone ?? scheduled ?? 'UTC';
    const counts = first?.counts;
    const refused =
        list.error instanceof ApiError && SEARCH_REFUSALS.has(list.error.code)
            ? errorMessage(t, list.error)
            : null;
    const canBan = allows(me, Permission.PlayersBan);
    const canLift = allows(me, Permission.BansLift);
    const countOf = (each: BanFilter) =>
        counts === undefined
            ? undefined
            : each === 'all'
              ? counts.active + counts.ended + counts.lifted
              : counts[each];
    const filterWords = Object.fromEntries(
        FILTERS.map((each) => {
            const value = countOf(each);
            return [each, value === undefined ? words.filters[each] : `${words.filters[each]} ${value}`];
        }),
    ) as Record<BanFilter, string>;

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {words.title} <span className="gloss">{words.gloss}</span>
                </h1>
                <div className="flex flex-wrap items-center gap-3 max-md:w-full">
                    <ZoneNote zone={zone} />
                    {canBan && (
                        <Button icon={Plus} className="max-md:flex-1" onClick={() => setBanning(true)}>
                            {words.banIdentifiers}
                        </Button>
                    )}
                </div>
            </div>
            {done !== null && <Notice tone={done.tone}>{done.message}</Notice>}
            <Card id="bans-list" icon={BanIcon} title={words.list} flush>
                <form
                    className="flex flex-wrap items-start gap-3 border-b border-line p-5 max-md:p-4"
                    onSubmit={(event) => {
                        event.preventDefault();
                        setSearch(draft.trim());
                    }}
                >
                    <div className="flex w-full">
                        <Segmented
                            label={words.filter}
                            choices={FILTERS}
                            value={filter}
                            words={filterWords}
                            onChange={setFilter}
                        />
                    </div>
                    <TextField
                        label={words.search}
                        hint={
                            allows(me, Permission.PlayerIdentifiersView)
                                ? words.searchHintIdentifiers
                                : words.searchHint
                        }
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
                    {list.isSuccess && bans.length === 0 && (
                        <p className="mx-auto max-w-empty px-6 py-6 text-center text-ui text-ink-3">
                            {search === '' ? words.empty[filter] : words.noMatch}
                        </p>
                    )}
                    {bans.length > 0 && refused === null && (
                        <>
                            <div className={cn(table.scroll, 'rounded-none max-md:hidden')}>
                                <table className={table.table}>
                                    <caption className="sr-only">
                                        {words.caption(words.filters[filter])}
                                    </caption>
                                    <thead>
                                        <tr>
                                            <th scope="col" className={table.th}>
                                                {words.player}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {words.reason}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {filter === 'lifted' ? words.lifted : words.ends}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {words.by}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {words.code}
                                            </th>
                                            {canLift && (
                                                <th scope="col" className={table.th}>
                                                    <span className="sr-only">{words.lift}</span>
                                                </th>
                                            )}
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {bans.map((ban) => (
                                            <BanRow
                                                key={ban.code}
                                                ban={ban}
                                                zone={zone}
                                                onLift={canLift ? () => setLifting(ban) : undefined}
                                            />
                                        ))}
                                    </tbody>
                                </table>
                            </div>
                            <ul className="flex flex-col md:hidden">
                                {bans.map((ban) => (
                                    <BanItem
                                        key={ban.code}
                                        ban={ban}
                                        zone={zone}
                                        onLift={canLift ? () => setLifting(ban) : undefined}
                                    />
                                ))}
                            </ul>
                        </>
                    )}
                </div>
                <div className="flex flex-wrap items-center gap-2 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3 max-md:pl-4">
                    <span className="mr-auto">{words.kept}</span>
                    {bans.length > 0 && refused === null && (
                        <output>
                            {list.hasNextPage ? words.shown(bans.length) : words.allShown(bans.length)}
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
            {banning && (
                <BanDialog
                    target={{ player: null }}
                    onClose={() => setBanning(false)}
                    onBanned={(ban) => {
                        setBanning(false);
                        setDone({ tone: 'success', message: words.banned(ban.code) });
                    }}
                />
            )}
            {lifting !== null && (
                <LiftDialog
                    ban={lifting}
                    onClose={() => setLifting(null)}
                    onLifted={() => {
                        setDone({ tone: 'success', message: words.liftedNotice(lifting.code) });
                        setLifting(null);
                    }}
                />
            )}
        </div>
    );
}

function Who({ ban }: { ban: Ban }) {
    const { t } = useI18n();
    const words = t.bans;
    return (
        <div className="flex min-w-0 flex-col gap-0.5">
            {ban.player !== null && ban.name !== null ? (
                <Link
                    to="/players/$id"
                    params={{ id: String(ban.player) }}
                    className="font-medium text-ink underline-offset-2 [overflow-wrap:anywhere] hover:underline"
                >
                    {ban.name}
                </Link>
            ) : (
                <span className="text-ink-2 italic">{ban.name ?? words.noProfile}</span>
            )}
            {ban.license !== null && (
                <CopyValue
                    value={ban.license}
                    shown={shortLicence(ban.license)}
                    label={t.players.copyLicence(ban.license)}
                />
            )}
            <span className="text-meta text-ink-3">
                {ban.player === null
                    ? words.heldTyped(ban.identifiers, ban.tokens)
                    : words.held(ban.identifiers, ban.tokens)}
            </span>
        </div>
    );
}

function BanRow({ ban, zone, onLift }: { ban: Ban; zone: string; onLift?: () => void }) {
    const { t } = useI18n();
    const visit = useVisit(zone);
    return (
        <tr className={table.row}>
            <td className={cn(table.td, 'min-w-40 align-top')}>
                <Who ban={ban} />
            </td>
            <td className={cn(table.td, 'max-w-80 min-w-40 align-top [overflow-wrap:anywhere]')}>
                {ban.reason}
                <div className={table.sub}>{t.bans.refused(ban.refused)}</div>
            </td>
            <td className={cn(table.td, 'min-w-32 align-top')}>
                <BanEnds ban={ban} zone={zone} />
            </td>
            <td className={cn(table.td, 'align-top whitespace-nowrap')}>
                <span className="block text-ink">{ban.by}</span>
                <span className="block font-mono text-small text-ink-3">{visit(ban.createdAt)}</span>
            </td>
            <td className={cn(table.td, 'align-top')}>
                <CopyValue value={ban.code} shown={ban.code} label={t.bans.copyCode} />
            </td>
            {onLift !== undefined && (
                <td className={cn(table.td, 'align-top text-right')}>
                    {ban.state === 'active' && (
                        <Button tone="ghost" size="sm" icon={Undo2} onClick={onLift}>
                            {t.bans.lift}
                        </Button>
                    )}
                </td>
            )}
        </tr>
    );
}

function BanItem({ ban, zone, onLift }: { ban: Ban; zone: string; onLift?: () => void }) {
    const { t } = useI18n();
    const words = t.bans;
    const visit = useVisit(zone);
    return (
        <li className="flex flex-col gap-2 border-b border-line px-4 py-3 last:border-b-0">
            <Who ban={ban} />
            <p className="text-ui text-ink-2 [overflow-wrap:anywhere]">{ban.reason}</p>
            <dl className="grid grid-cols-2 gap-x-4 gap-y-2 text-small">
                <div className="flex min-w-0 flex-col gap-0.5">
                    <dt className="text-meta text-ink-3">
                        {ban.lifted === null ? words.ends : words.lifted}
                    </dt>
                    <dd>
                        <BanEnds ban={ban} zone={zone} />
                    </dd>
                </div>
                <div className="flex min-w-0 flex-col gap-0.5">
                    <dt className="text-meta text-ink-3">{words.by}</dt>
                    <dd className="text-ink">
                        {ban.by}
                        <span className="block font-mono text-ink-3">{visit(ban.createdAt)}</span>
                    </dd>
                </div>
                <div className="flex min-w-0 flex-col gap-0.5">
                    <dt className="text-meta text-ink-3">{words.code}</dt>
                    <dd>
                        <CopyValue value={ban.code} shown={ban.code} label={words.copyCode} />
                    </dd>
                </div>
            </dl>
            {onLift !== undefined && ban.state === 'active' && (
                <Button icon={Undo2} onClick={onLift}>
                    {words.lift}
                </Button>
            )}
        </li>
    );
}
