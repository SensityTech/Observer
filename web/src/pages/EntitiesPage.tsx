import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { Link, useNavigate, useSearch } from '@tanstack/react-router';
import {
    ArrowDown,
    ArrowUp,
    Boxes,
    ChevronLeft,
    ChevronRight,
    Info,
    Search,
    Trash2,
    User,
} from 'lucide-react';
import { useMemo, useState, type ReactNode } from 'react';

import { entitiesQuery } from '../api/entities';
import type { EntitiesLive } from '../api/generated/EntitiesLive';
import type { EntityItem } from '../api/generated/EntityItem';
import type { EntityKind } from '../api/generated/EntityKind';
import type { EntityPage } from '../api/generated/EntityPage';
import type { EntityResult } from '../api/generated/EntityResult';
import type { EntitySort } from '../api/generated/EntitySort';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Checkbox } from '../components/Checkbox';
import { Chip } from '../components/Chip';
import { TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { SelectField } from '../components/SelectField';
import { table } from '../components/table';
import { Toggletip } from '../components/Toggletip';
import { DeleteDialog } from '../entities/DeleteDialog';
import { useEntitiesLive } from '../entities/live';
import { EntityMark, entityName, kindWord } from '../entities/marks';
import { blocker, isStale, keyOf, modelHash } from '../entities/model';
import {
    CLASSES,
    filtered,
    KINDS,
    listingOf,
    MAX_SEARCH,
    parseEntitiesSearch,
    withFilters,
    withPage,
    withSort,
    type EntitiesSearch,
} from '../entities/search';
import { NOTHING, pageState, pageToggled, refreshed, toggled, type Selection } from '../entities/selection';
import { deletionNotice } from '../entities/words';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useNow } from '../lib/useNow';
import { useServerStatus } from '../server/context';

type KindChoice = EntityKind | 'all';
const KIND_CHOICES: KindChoice[] = ['all', ...KINDS];
const RUNNING = new Set(['starting', 'online', 'degraded', 'hung']);
const COLUMNS: { sort: EntitySort; key: 'kind' | 'model' | 'resource' | 'owner' | 'position' }[] = [
    { sort: 'kind', key: 'kind' },
    { sort: 'model', key: 'model' },
    { sort: 'resource', key: 'resource' },
    { sort: 'owner', key: 'owner' },
    { sort: 'position', key: 'position' },
];
/** How many of the most common models the filters offer. */
const MODELS_SHOWN = 3;

type Done = { tone: 'success' | 'warning'; message: string };

/** The server's vehicles, peds and objects: found by what they are, and deleted a few at a time. */
export function EntitiesPage() {
    const { t, format } = useI18n();
    const words = t.entities;
    const me = useMe();
    const navigate = useNavigate();
    const raw = useSearch({ strict: false });
    const search = useMemo(() => parseEntitiesSearch(raw as Record<string, unknown>), [raw]);
    const list = useQuery({ ...entitiesQuery(listingOf(search)), placeholderData: keepPreviousData });
    // Following the topic is what has the server list its entities while the page is open.
    const followed = useEntitiesLive();
    const status = useServerStatus().status;
    const now = useNow(1_000);
    const [selection, setSelection] = useState<Selection>(NOTHING);
    const [deleting, setDeleting] = useState<EntityItem[] | null>(null);
    const [done, setDone] = useState<Done | null>(null);
    const [draft, setDraft] = useState(search.search ?? '');
    const [typedFor, setTypedFor] = useState(search.search);
    if (typedFor !== search.search) {
        setTypedFor(search.search);
        setDraft(search.search ?? '');
    }

    const page = list.data;
    const live: EntitiesLive | null = followed ?? page?.live ?? null;
    const run = live?.run ?? null;
    // A list read again keeps the selection as it now is; another start of the server empties it.
    const [seen, setSeen] = useState<{ page: EntityPage | undefined; run: string | null }>({ page, run });
    if (seen.page !== page || seen.run !== run) {
        setSeen({ page, run });
        setSelection(seen.run !== run ? NOTHING : refreshed(selection, page?.items ?? []));
    }

    const items = page?.items ?? [];
    const limits = page?.limits;
    const facets = page?.facets;
    const ownersShown = page?.ownersShown ?? false;
    const canDelete = allows(me, Permission.EntitiesDelete);
    const stale = live !== null && limits !== undefined && isStale(live, now, limits.staleSweeps);
    const running = status === null || RUNNING.has(status.state);
    const deletable = canDelete && !stale && run !== null && limits !== undefined;
    const most = limits?.maxDelete ?? 0;
    const total =
        facets === undefined ? null : facets.kinds.vehicles + facets.kinds.peds + facets.kinds.objects;
    const countOf = (kind: KindChoice) =>
        facets === undefined
            ? undefined
            : kind === 'all'
              ? (total ?? undefined)
              : facets.kinds[kind === 'vehicle' ? 'vehicles' : kind === 'ped' ? 'peds' : 'objects'];
    const kindWords = Object.fromEntries(
        KIND_CHOICES.map((kind) => {
            const value = countOf(kind);
            return [
                kind,
                value === undefined ? words.filters[kind] : `${words.filters[kind]} ${format.number(value)}`,
            ];
        }),
    ) as Record<KindChoice, string>;

    const go = (next: EntitiesSearch, replace = false) =>
        void navigate({ to: '/entities', search: next, replace, resetScroll: false });
    const narrow = (change: Partial<EntitiesSearch>) => go(withFilters(search, { ...search, ...change }));
    const finished = (results: EntityResult[]) => {
        setDone(deletionNotice(t, results));
        setDeleting(null);
        setSelection(NOTHING);
    };

    const unavailable = live?.unavailable ?? null;
    const empty: ReactNode = !running ? (
        <Empty title={words.stopped.title}>{words.stopped.body}</Empty>
    ) : unavailable === 'onesync' ? (
        <Empty title={words.unavailable.onesync.title}>
            {words.unavailable.onesync.body}{' '}
            {me.isOwner ? (
                <Link
                    to="/settings/$category"
                    params={{ category: 'launch' }}
                    className="font-medium text-ink underline underline-offset-2"
                >
                    {words.unavailable.onesync.link}
                </Link>
            ) : (
                words.unavailable.onesync.link
            )}
            .
        </Empty>
    ) : unavailable === 'game' ? (
        <Empty title={words.unavailable.game.title}>{words.unavailable.game.body}</Empty>
    ) : null;

    const first = page === undefined ? 0 : page.page * page.limits.pageSize;
    const state = pageState(selection, items);

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {words.title} <span className="gloss">{words.gloss}</span>
                </h1>
            </div>
            {stale && live?.at != null && (
                <Notice tone="warning" live={false}>
                    {words.stale(format.duration(Math.max(0, now - Date.parse(live.at)) / 1000))}
                </Notice>
            )}
            {live !== null && live.spaced && empty === null && (
                <Notice tone="warning" live={false}>
                    {words.spaced(Math.round(live.everyMs / 1000))}
                </Notice>
            )}
            {live !== null && live.truncated && empty === null && (
                <Notice live={false}>{words.truncated(live.listed)}</Notice>
            )}
            {done !== null && <Notice tone={done.tone}>{done.message}</Notice>}
            <Card
                id="entities-list"
                icon={Boxes}
                title={total === null || empty !== null ? words.list : words.count(total)}
                meta={
                    empty === null && live?.counts != null && live.counts.playerPeds > 0
                        ? words.playerPeds(live.counts.playerPeds)
                        : undefined
                }
                flush
            >
                {empty ?? (
                    <>
                        <form
                            className="flex flex-wrap items-start gap-3 border-b border-line p-5 max-md:p-4"
                            onSubmit={(event) => {
                                event.preventDefault();
                                narrow({ search: draft });
                            }}
                        >
                            <div className="flex w-full">
                                <Segmented
                                    label={words.filter}
                                    choices={KIND_CHOICES}
                                    value={search.kind ?? 'all'}
                                    words={kindWords}
                                    onChange={(kind) =>
                                        narrow({
                                            kind: kind === 'all' ? undefined : kind,
                                            class: kind === 'vehicle' ? search.class : undefined,
                                        })
                                    }
                                />
                            </div>
                            <TextField
                                label={words.search}
                                type="search"
                                value={draft}
                                maxLength={MAX_SEARCH}
                                placeholder={ownersShown ? words.searchHint : words.searchHintNoOwner}
                                spellCheck={false}
                                autoCapitalize="none"
                                autoComplete="off"
                                className="min-w-0 flex-1 basis-56"
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
                            <button type="submit" className="sr-only">
                                {words.submit}
                            </button>
                            {search.kind === 'vehicle' && (
                                <SelectField
                                    label={words.class}
                                    className="basis-30 max-md:flex-1 max-md:basis-[45%]"
                                    narrowed={search.class !== undefined}
                                    value={search.class ?? ''}
                                    choices={[
                                        { value: '', label: words.any },
                                        ...CLASSES.filter(
                                            (each) =>
                                                each === search.class ||
                                                facets?.classes.some((counted) => counted.class === each),
                                        ).map((each) => ({ value: each, label: words.classes[each] })),
                                    ]}
                                    onChange={(value) => narrow({ class: value === '' ? undefined : value })}
                                />
                            )}
                            <SelectField
                                label={words.createdBy}
                                className="basis-40 max-md:flex-1 max-md:basis-[45%]"
                                narrowed={search.resource !== undefined}
                                value={
                                    search.resource === undefined
                                        ? 'any'
                                        : search.resource === ''
                                          ? 'none'
                                          : `name:${search.resource}`
                                }
                                choices={[
                                    { value: 'any', label: words.anyResource },
                                    { value: 'none', label: words.noResource },
                                    ...resourceNames(facets, search.resource).map((name) => ({
                                        value: `name:${name}`,
                                        label: name,
                                    })),
                                ]}
                                onChange={(value) =>
                                    narrow({
                                        resource:
                                            value === 'any'
                                                ? undefined
                                                : value === 'none'
                                                  ? ''
                                                  : value.slice('name:'.length),
                                    })
                                }
                            />
                            <SelectField
                                label={words.owner}
                                className="basis-30 max-md:flex-1 max-md:basis-[45%]"
                                narrowed={search.owner !== undefined}
                                value={String(search.owner ?? 'any')}
                                choices={[
                                    { value: 'any', label: words.anyone },
                                    { value: 'none', label: words.noOwner },
                                    { value: 'owned', label: words.someOwner },
                                    ...ownerIds(facets, search.owner).map(({ netId, name }) => ({
                                        value: String(netId),
                                        label: words.player(netId, name),
                                    })),
                                ]}
                                onChange={(value) =>
                                    narrow({
                                        owner:
                                            value === 'any'
                                                ? undefined
                                                : value === 'none' || value === 'owned'
                                                  ? value
                                                  : Number(value),
                                    })
                                }
                            />
                            <SelectField
                                label={words.bucket}
                                className="basis-22 max-md:flex-1 max-md:basis-[45%]"
                                narrowed={search.bucket !== undefined}
                                value={String(search.bucket ?? 'any')}
                                choices={[
                                    { value: 'any', label: words.any },
                                    ...bucketIds(facets, search.bucket).map((bucket) => ({
                                        value: String(bucket),
                                        label: String(bucket),
                                    })),
                                ]}
                                onChange={(value) =>
                                    narrow({ bucket: value === 'any' ? undefined : Number(value) })
                                }
                            />
                            {((facets?.models.length ?? 0) > 0 || filtered(search)) && (
                                <div className="flex w-full flex-wrap items-center gap-2 text-meta text-ink-3">
                                    {(facets?.models.length ?? 0) > 0 && (
                                        <span className="max-md:hidden">{words.mostCommon}</span>
                                    )}
                                    {facets?.models.slice(0, MODELS_SHOWN).map((counted) => (
                                        <button
                                            key={`${counted.kind}:${counted.model}`}
                                            type="button"
                                            aria-label={words.showModel(modelHash(counted.model))}
                                            className="inline-flex h-5.5 items-center rounded-pill border border-line bg-surface-2 px-2 font-mono text-meta leading-none font-medium text-ink-2 [font-variant-ligatures:none] hover:border-line-strong hover:text-ink max-md:hidden"
                                            onClick={() => narrow({ search: modelHash(counted.model) })}
                                        >
                                            {words.modelCount(modelHash(counted.model), counted.count)}
                                        </button>
                                    ))}
                                    {filtered(search) && (
                                        <Button
                                            tone="ghost"
                                            size="sm"
                                            className="ml-auto"
                                            onClick={() => go(withFilters(search, {}))}
                                        >
                                            {words.clear}
                                        </Button>
                                    )}
                                </div>
                            )}
                        </form>
                        <div
                            aria-busy={list.isFetching}
                            className={cn(list.isPlaceholderData && 'opacity-50')}
                        >
                            {list.isPending && (
                                <div className="flex flex-col gap-3 p-5">
                                    <span className="sr-only">{words.loading}</span>
                                    <span className="skeleton w-3/4" />
                                    <span className="skeleton w-2/3" />
                                    <span className="skeleton w-1/2" />
                                </div>
                            )}
                            {list.isError && (
                                <div className="p-5 max-md:p-4">
                                    <Notice tone="danger">
                                        {words.failed} {errorMessage(t, list.error)}
                                    </Notice>
                                </div>
                            )}
                            {list.isSuccess && items.length === 0 && (
                                <p className="mx-auto max-w-empty px-6 py-6 text-center text-ui text-ink-3">
                                    {live?.at == null
                                        ? words.waiting
                                        : filtered(search)
                                          ? words.noMatch
                                          : words.none}
                                </p>
                            )}
                            {items.length > 0 && (
                                <>
                                    <div className={cn(table.scroll, 'rounded-none max-md:hidden')}>
                                        <table className={table.table}>
                                            <caption className="sr-only">{words.caption}</caption>
                                            <thead>
                                                <tr>
                                                    {canDelete && (
                                                        <th scope="col" className={cn(table.th, 'w-11')}>
                                                            <Checkbox
                                                                aria-label={words.selectPage}
                                                                checked={state === 'all'}
                                                                indeterminate={state === 'some'}
                                                                disabled={!deletable}
                                                                onChange={() =>
                                                                    setSelection(
                                                                        pageToggled(selection, items, most),
                                                                    )
                                                                }
                                                            />
                                                        </th>
                                                    )}
                                                    {COLUMNS.map(({ sort, key }) => {
                                                        const current = (search.sort ?? 'model') === sort;
                                                        const descending =
                                                            current && search.descending === true;
                                                        const Arrow = descending ? ArrowDown : ArrowUp;
                                                        return (
                                                            <th
                                                                key={sort}
                                                                scope="col"
                                                                className={table.th}
                                                                aria-sort={
                                                                    current
                                                                        ? descending
                                                                            ? 'descending'
                                                                            : 'ascending'
                                                                        : undefined
                                                                }
                                                            >
                                                                <button
                                                                    type="button"
                                                                    className={cn(
                                                                        'inline-flex items-center gap-1 rounded-xs font-medium hover:text-ink',
                                                                        current && 'text-ink',
                                                                    )}
                                                                    onClick={() => go(withSort(search, sort))}
                                                                >
                                                                    {words.columns[key]}
                                                                    {current && (
                                                                        <Arrow
                                                                            size={12}
                                                                            strokeWidth={2}
                                                                            aria-hidden="true"
                                                                        />
                                                                    )}
                                                                </button>
                                                            </th>
                                                        );
                                                    })}
                                                    <th scope="col" className={table.th}>
                                                        <span className="sr-only">{words.actions}</span>
                                                    </th>
                                                </tr>
                                            </thead>
                                            <tbody>
                                                {items.map((item) => {
                                                    const picked = selection.has(keyOf(item));
                                                    const never = blocker(item);
                                                    return (
                                                        <tr
                                                            key={keyOf(item)}
                                                            className={cn(
                                                                table.row,
                                                                picked &&
                                                                    'bg-accent-soft hover:bg-accent-soft',
                                                            )}
                                                        >
                                                            {canDelete && (
                                                                <td className={table.td}>
                                                                    <Checkbox
                                                                        aria-label={words.select(
                                                                            entityName(t, item),
                                                                        )}
                                                                        title={
                                                                            never === null
                                                                                ? undefined
                                                                                : words.never[never]
                                                                        }
                                                                        checked={picked}
                                                                        disabled={
                                                                            !deletable ||
                                                                            never !== null ||
                                                                            (!picked &&
                                                                                selection.size >= most)
                                                                        }
                                                                        onChange={() =>
                                                                            setSelection(
                                                                                toggled(
                                                                                    selection,
                                                                                    item,
                                                                                    most,
                                                                                ),
                                                                            )
                                                                        }
                                                                    />
                                                                </td>
                                                            )}
                                                            <td className={table.td}>
                                                                <Kind item={item} />
                                                            </td>
                                                            <td className={table.td}>
                                                                <Model item={item} />
                                                            </td>
                                                            <td className={table.td}>
                                                                <Creator item={item} />
                                                            </td>
                                                            <td className={table.td}>
                                                                <Owner item={item} shown={ownersShown} />
                                                            </td>
                                                            <td className={table.td}>
                                                                <span className={table.mono}>
                                                                    {place(item)}
                                                                </span>
                                                                {item.bucket !== 0 && (
                                                                    <div className={table.sub}>
                                                                        {words.bucketOf(item.bucket)}
                                                                    </div>
                                                                )}
                                                            </td>
                                                            <td className={cn(table.td, 'text-right')}>
                                                                <div className="flex justify-end gap-0.5">
                                                                    {canDelete && (
                                                                        <Button
                                                                            tone="ghost"
                                                                            size="icon-sm"
                                                                            aria-label={words.deleteOne(
                                                                                entityName(t, item),
                                                                            )}
                                                                            title={
                                                                                never === null
                                                                                    ? undefined
                                                                                    : words.never[never]
                                                                            }
                                                                            disabled={
                                                                                !deletable || never !== null
                                                                            }
                                                                            onClick={() =>
                                                                                setDeleting([item])
                                                                            }
                                                                        >
                                                                            <Trash2
                                                                                size={16}
                                                                                strokeWidth={1.5}
                                                                                aria-hidden="true"
                                                                            />
                                                                        </Button>
                                                                    )}
                                                                </div>
                                                            </td>
                                                        </tr>
                                                    );
                                                })}
                                            </tbody>
                                        </table>
                                    </div>
                                    <ul className="flex flex-col md:hidden">
                                        {items.map((item) => {
                                            const picked = selection.has(keyOf(item));
                                            const never = blocker(item);
                                            return (
                                                <li
                                                    key={keyOf(item)}
                                                    className={cn(
                                                        'flex items-start gap-3 border-b border-line px-4 py-3 last:border-b-0',
                                                        picked && 'bg-accent-soft',
                                                    )}
                                                >
                                                    {canDelete && (
                                                        <Checkbox
                                                            aria-label={words.select(entityName(t, item))}
                                                            className="mt-0.5"
                                                            checked={picked}
                                                            disabled={
                                                                !deletable ||
                                                                never !== null ||
                                                                (!picked && selection.size >= most)
                                                            }
                                                            onChange={() =>
                                                                setSelection(toggled(selection, item, most))
                                                            }
                                                        />
                                                    )}
                                                    <div className="flex min-w-0 flex-1 flex-col gap-0.75">
                                                        <span className="flex items-center gap-2 text-nav font-medium text-ink">
                                                            <EntityMark kind={item.kind} />
                                                            {kindWord(t, item)}
                                                            <span className="font-mono text-small font-normal [font-variant-ligatures:none]">
                                                                {modelHash(item.model)}
                                                            </span>
                                                        </span>
                                                        <span className="flex flex-wrap items-center gap-x-1.5 text-small text-ink-3">
                                                            <Creator item={item} plain />
                                                            <span aria-hidden="true">·</span>
                                                            <Owner item={item} shown={ownersShown} plain />
                                                        </span>
                                                        <span className="flex flex-wrap items-center gap-x-1.5 gap-y-1 font-mono text-meta text-ink-3">
                                                            {[
                                                                place(item),
                                                                words.netId(item.netId),
                                                                ...(item.bucket === 0
                                                                    ? []
                                                                    : [words.bucketOf(item.bucket)]),
                                                            ].join(' · ')}
                                                            {item.occupied && (
                                                                <Chip tone="accent" className="font-sans">
                                                                    {words.playerInside}
                                                                </Chip>
                                                            )}
                                                        </span>
                                                    </div>
                                                </li>
                                            );
                                        })}
                                    </ul>
                                </>
                            )}
                        </div>
                        {selection.size > 0 && (
                            <div className="pointer-events-none sticky bottom-3 z-(--z-dock) flex justify-center px-3 py-3 max-md:bottom-0 max-md:p-0">
                                <div className="pointer-events-auto flex max-w-full flex-wrap items-center gap-3 rounded-box border border-line-strong bg-raise py-1.5 pr-1.5 pl-4 text-ui text-ink shadow-pop max-md:w-full max-md:rounded-none max-md:border-x-0 max-md:border-b-0 max-md:py-2 max-md:pr-2">
                                    <output className="mr-auto">
                                        <span className="font-semibold">
                                            {words.selected(selection.size)}
                                        </span>{' '}
                                        <span className="text-meta text-ink-3">· {words.atMost(most)}</span>
                                    </output>
                                    <Button tone="ghost" size="sm" onClick={() => setSelection(NOTHING)}>
                                        {words.clearSelection}
                                    </Button>
                                    <Button
                                        tone="danger"
                                        size="sm"
                                        icon={Trash2}
                                        disabled={!deletable}
                                        onClick={() => setDeleting([...selection.values()])}
                                    >
                                        {words.deleteSelected(selection.size)}
                                    </Button>
                                </div>
                            </div>
                        )}
                        <div className="flex flex-wrap items-center gap-2 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3 max-md:pl-4">
                            <span className="mr-auto flex items-center gap-1">
                                {live?.at == null
                                    ? words.waiting
                                    : words.listed(
                                          format.duration(Math.max(0, now - Date.parse(live.at)) / 1000),
                                          Math.round(live.everyMs / 1000),
                                      )}
                                {live?.workUs != null && live.frames !== null && (
                                    <Toggletip
                                        tip={words.costOf(
                                            format.milliseconds(live.workUs / 1000),
                                            live.frames,
                                        )}
                                        className="inline-grid size-5 place-items-center rounded-full text-ink-3 hover:text-ink"
                                    >
                                        <Info size={14} strokeWidth={1.5} aria-hidden="true" />
                                        <span className="sr-only">{words.cost}</span>
                                    </Toggletip>
                                )}
                            </span>
                            {page !== undefined && items.length > 0 && (
                                <>
                                    <output>
                                        {words.range(first + 1, first + items.length, page.matching)}
                                    </output>
                                    <span className="inline-flex items-center gap-1">
                                        <Button
                                            tone="ghost"
                                            size="sm"
                                            icon={ChevronLeft}
                                            disabled={page.page === 0}
                                            onClick={() => go(withPage(search, page.page))}
                                        >
                                            {words.previous}
                                        </Button>
                                        <Button
                                            tone="ghost"
                                            size="sm"
                                            disabled={first + items.length >= page.matching}
                                            onClick={() => go(withPage(search, page.page + 2))}
                                        >
                                            {words.next}
                                            <ChevronRight size={14} strokeWidth={1.5} aria-hidden="true" />
                                        </Button>
                                    </span>
                                </>
                            )}
                        </div>
                    </>
                )}
            </Card>
            {deleting !== null && run !== null && limits !== undefined && (
                <DeleteDialog
                    items={deleting}
                    run={run}
                    limits={limits}
                    onClose={() => setDeleting(null)}
                    onDone={finished}
                />
            )}
        </div>
    );
}

/** The resources the filter offers: those the list counts, and the one the address names. */
const resourceNames = (facets: EntityPage['facets'] | undefined, chosen: string | undefined) => {
    const names = (facets?.resources ?? []).flatMap(({ resource }) => (resource === null ? [] : [resource]));
    return chosen === undefined || chosen === '' || names.includes(chosen) ? names : [chosen, ...names];
};

const ownerIds = (facets: EntityPage['facets'] | undefined, chosen: EntitiesSearch['owner']) => {
    const owners = (facets?.owners ?? []).map(({ netId, name }) => ({ netId, name }));
    return typeof chosen !== 'number' || owners.some((owner) => owner.netId === chosen)
        ? owners
        : [{ netId: chosen, name: null }, ...owners];
};

const bucketIds = (facets: EntityPage['facets'] | undefined, chosen: number | undefined) => {
    const buckets = (facets?.buckets ?? []).map(({ bucket }) => bucket);
    return chosen === undefined || buckets.includes(chosen) ? buckets : [chosen, ...buckets];
};

const place = (item: Pick<EntityItem, 'x' | 'y'>) => `${item.x.toFixed(1)}, ${item.y.toFixed(1)}`;

function Empty({ title, children }: { title: string; children: ReactNode }) {
    return (
        <div className="mx-auto flex max-w-[26rem] flex-col items-center gap-1.5 px-6 py-7 text-center text-ui text-ink-3">
            <b className="text-card text-ink">{title}</b>
            <p>{children}</p>
        </div>
    );
}

function Kind({ item }: { item: EntityItem }) {
    const { t } = useI18n();
    return (
        <span className="inline-flex items-center gap-2.25 whitespace-nowrap text-ink">
            <EntityMark kind={item.kind} />
            {kindWord(t, item)}
        </span>
    );
}

function Model({ item }: { item: EntityItem }) {
    const { t } = useI18n();
    return (
        <>
            <span className={table.mono}>{modelHash(item.model)}</span>
            <div className={table.sub}>
                <span className="font-mono">{t.entities.netId(item.netId)}</span>
                {item.occupied && <Chip tone="accent">{t.entities.playerInside}</Chip>}
            </div>
        </>
    );
}

/** The resource that created the entity, none, or a lookup the server has not made yet. */
function Creator({ item, plain = false }: { item: EntityItem; plain?: boolean }) {
    const { t } = useI18n();
    if (item.resource !== null) {
        return <span className={cn('[overflow-wrap:anywhere]', !plain && 'text-ink')}>{item.resource}</span>;
    }
    return (
        <span className="text-ink-3">
            {item.resourcePending ? t.entities.lookingUp : t.entities.noResource}
        </span>
    );
}

/** Whose game simulates the entity; only that there is one, for who does not see where players stand. */
function Owner({ item, shown, plain = false }: { item: EntityItem; shown: boolean; plain?: boolean }) {
    const { t } = useI18n();
    if (!item.owned) {
        return <span className="text-ink-3">{t.entities.noOwner}</span>;
    }
    return (
        <span className={cn('inline-flex min-w-0 items-center gap-1.5', !plain && 'text-ink')}>
            {!plain && (
                <User size={14} strokeWidth={1.5} className="flex-none text-ink-3" aria-hidden="true" />
            )}
            {shown && item.owner !== null ? (
                <>
                    <span className="font-mono text-meta text-ink-3">{t.entities.netId(item.owner)}</span>
                    {item.ownerName !== null && (
                        <span className="[overflow-wrap:anywhere]">{item.ownerName}</span>
                    )}
                </>
            ) : (
                t.entities.someOwner
            )}
        </span>
    );
}
