import { useQuery } from '@tanstack/react-query';
import { Link, useSearch } from '@tanstack/react-router';
import {
    Ban as BanIcon,
    ExternalLink,
    Image as ImageIcon,
    LocateFixed,
    Map as MapIcon,
    RotateCw,
    Search,
    X,
} from 'lucide-react';
import { useId, useMemo, useState, type ReactNode } from 'react';

import type { EntityItem } from '../api/generated/EntityItem';
import type { MapBounds } from '../api/generated/MapBounds';
import type { MapLive } from '../api/generated/MapLive';
import type { MapPlayer } from '../api/generated/MapPlayer';
import type { MapSettings } from '../api/generated/MapSettings';
import { Permission } from '../api/generated/Permission';
import { mapQuery, useMapImage } from '../api/map';
import { onlineQuery } from '../api/players';
import { allows, useMe } from '../auth/me';
import { BanDialog } from '../bans/dialogs';
import { Button, buttonVariants } from '../components/Button';
import { Chip } from '../components/Chip';
import { FieldError, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { Switch } from '../components/Switch';
import { DeleteDialog } from '../entities/DeleteDialog';
import { around } from '../entities/layer';
import { EntityCanvas, EntityCard, LayerBar, useEntityLayer } from '../entities/MapLayer';
import { deletionNotice } from '../entities/words';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { WIDE, useMedia } from '../lib/useMedia';
import { useNow } from '../lib/useNow';
import { useTopic } from '../live/context';
import { tierOf, type Tier } from '../map/geometry';
import { MapCanvas, TIER_MARK, type Dot } from '../map/MapCanvas';
import type { MapSearch } from '../map/search';
import { TILE_STYLES, isTileStyle, type TileStyle } from '../map/tiles';
import { extend, type Trails } from '../map/trails';
import { useServerStatus } from '../server/context';

/** Until the settings are read: the grid, so that nothing is asked of Rockstar's servers unasked. */
const DEFAULTS: MapSettings = {
    imagery: 'grid',
    bounds: null,
    newHours: 10,
    veteranHours: 100,
    everySeconds: 2,
};
const TIERS: Tier[] = ['new', 'regular', 'veteran', 'unconfirmed'];
/** The style a viewer picked for themselves, among Rockstar's, kept in their browser. */
const STYLE_KEY = 'observer.mapStyle';

const storedStyle = (): TileStyle | null => {
    try {
        const stored = localStorage.getItem(STYLE_KEY);
        return TILE_STYLES.find((style) => style === stored) ?? null;
    } catch {
        return null;
    }
};

export function MapPage() {
    const { t, format } = useI18n();
    const words = t.map;
    const me = useMe();
    const view = useQuery(mapQuery);
    const online = useQuery(onlineQuery);
    const address: MapSearch = useSearch({ strict: false });
    const layer = useEntityLayer(allows(me, Permission.EntitiesView), address.entity);
    const [frame, setFrame] = useState<(MapBounds & { at: number }) | null>(null);
    const [deleting, setDeleting] = useState<EntityItem | null>(null);
    const [deleted, setDeleted] = useState<ReturnType<typeof deletionNotice> | null>(null);
    // The entity the address names is brought into view once the list has it.
    const [broughtFor, setBroughtFor] = useState<number | null>(null);
    if (
        address.entity !== undefined &&
        layer.item?.netId === address.entity &&
        broughtFor !== address.entity
    ) {
        setBroughtFor(address.entity);
        setFrame({ ...around(layer.item.x, layer.item.y), at: address.entity });
    }
    const status = useServerStatus().status;
    const [live, setLive] = useState<MapLive | null>(null);
    const [trails, setTrails] = useState<Trails>(new Map());
    const took = (next: MapLive) => {
        setLive(next);
        setTrails((current) => extend(current, next.players, Date.now()));
    };
    const lost = () => {
        setLive(null);
        setTrails(new Map());
    };
    useTopic('server.map', { snapshot: took, event: took, refused: lost, cleared: lost });
    const listId = useId();
    const beside = useMedia(WIDE);
    const settings = view.data?.settings ?? DEFAULTS;
    const image = view.data?.image ?? null;
    const imageUrl = useMapImage(image?.version ?? null);
    const [selected, setSelected] = useState<number | null>(null);
    const [names, setNames] = useState(false);
    const [trailing, setTrailing] = useState(false);
    const [following, setFollowing] = useState(false);
    const [style, setStyle] = useState<TileStyle | null>(storedStyle);
    const [find, setFind] = useState('');
    const [missed, setMissed] = useState(false);
    const [focus, setFocus] = useState<{ id: number; at: number } | null>(null);
    const [banning, setBanning] = useState<MapPlayer | null>(null);
    // The style Rockstar's servers did not send, until another is chosen or the page asks again.
    const [unsent, setUnsent] = useState<string | null>(null);
    const now = useNow(1_000);

    const players = useMemo(() => live?.players ?? [], [live]);
    const nameOf = (player: MapPlayer) => player.label?.name ?? player.name ?? words.unnamed(player.netId);
    const dots: Dot[] = players.map((player) => {
        const tier = tierOf(player.player === null ? null : player.totalMs, settings);
        return {
            id: player.netId,
            x: player.x,
            y: player.y,
            tier,
            name: nameOf(player),
            label: words.dot(
                nameOf(player),
                words.tiers[tier],
                player.totalMs === null ? null : format.playTime(player.totalMs),
            ),
        };
    });
    const chosen = players.find((player) => player.netId === selected) ?? null;
    const session = online.data?.players.find((player) => player.netId === selected) ?? null;
    // Rockstar's map is the owner's choice; which of its styles shows is each viewer's.
    const imagery = isTileStyle(settings.imagery) ? (style ?? settings.imagery) : settings.imagery;
    const tiles = isTileStyle(imagery) && unsent !== imagery ? imagery : null;
    const pings = new Map(online.data?.players.map((player) => [player.netId, player.ping]));
    const wanted = find.trim().toLowerCase();
    const listed = players
        .filter(
            (player) =>
                wanted === '' ||
                nameOf(player).toLowerCase().includes(wanted) ||
                (player.name ?? '').toLowerCase().includes(wanted) ||
                String(player.netId) === wanted,
        )
        .toSorted((left, right) => nameOf(left).localeCompare(nameOf(right)));

    // Following keeps the map on the selected player: each new position is a new ask to show them.
    const followed = following && chosen !== null ? chosen.netId : null;
    const followFocus = followed === null ? null : { id: followed, at: Date.parse(live?.at ?? '') || 0 };
    const placedImage =
        imagery === 'image' && image !== null && imageUrl !== null && settings.bounds !== null
            ? { url: imageUrl, width: image.width, height: image.height }
            : null;
    const running = status !== null && ['starting', 'online', 'degraded', 'hung'].includes(status.state);
    const canBan = allows(me, Permission.PlayersBan);

    const show = (netId: number) => {
        setSelected(netId);
        layer.pick(null);
        setFocus({ id: netId, at: Date.now() });
    };
    /** Enter in the search goes to the first player it leaves. */
    const search = () => {
        const [found] = listed;
        setMissed(wanted !== '' && found === undefined);
        if (wanted !== '' && found !== undefined) {
            show(found.netId);
        }
    };

    // Beside the map when the list is, so that it never covers what it is about.
    const card =
        chosen === null ? (
            layer.picked === null ? null : (
                <EntityCard
                    layer={layer}
                    beside={beside}
                    canDelete={allows(me, Permission.EntitiesDelete)}
                    onDelete={setDeleting}
                />
            )
        ) : (
            <section
                data-overlay={beside ? undefined : true}
                aria-label={words.selected}
                className={
                    beside
                        ? 'flex flex-col gap-3 border-b border-line p-4'
                        : 'absolute top-14 right-3 flex w-72 flex-col gap-3 rounded-box border border-line-strong bg-surface p-3.5 shadow-pop max-md:inset-x-2 max-md:top-auto max-md:bottom-2 max-md:w-auto'
                }
            >
                <div className="flex items-start gap-2.5">
                    <span
                        aria-hidden="true"
                        className={cn(
                            'mt-1.5 inline-block flex-none',
                            TIER_MARK[tierOf(chosen.player === null ? null : chosen.totalMs, settings)],
                        )}
                    />
                    <div className="flex min-w-0 flex-1 flex-col">
                        <span className="text-card [overflow-wrap:anywhere] text-ink">
                            {chosen.name ?? words.unnamed(chosen.netId)}
                        </span>
                        {chosen.label !== null && (
                            <span className="text-meta text-ink-3">
                                {[chosen.label.name, chosen.label.id].filter(Boolean).join(' · ')}
                            </span>
                        )}
                    </div>
                    <Button
                        tone="ghost"
                        size="icon-sm"
                        aria-label={words.close}
                        onClick={() => {
                            setSelected(null);
                            setFollowing(false);
                        }}
                    >
                        <X size={14} strokeWidth={1.5} aria-hidden="true" />
                    </Button>
                </div>
                <div className="flex flex-wrap gap-1.5">
                    <Chip>
                        {words.tiers[tierOf(chosen.player === null ? null : chosen.totalMs, settings)]}
                    </Chip>
                    <Chip className="font-mono">#{chosen.netId}</Chip>
                </div>
                <dl className="grid grid-cols-2 gap-x-3 gap-y-2">
                    <Fact term={words.playTime}>
                        {chosen.totalMs === null ? '—' : format.playTime(chosen.totalMs)}
                    </Fact>
                    <Fact term={words.session}>
                        {session === null ? '—' : format.playTime(session.sessionMs)}
                    </Fact>
                    <Fact term={words.ping}>
                        {session?.ping == null ? words.unknownPing : format.milliseconds(session.ping)}
                    </Fact>
                    <Fact term={words.position}>
                        {`${chosen.x.toFixed(1)}, ${chosen.y.toFixed(1)}, ${chosen.z.toFixed(1)}`}
                    </Fact>
                </dl>
                <div className="flex flex-wrap gap-2 [&>*]:flex-1">
                    <Button
                        size="sm"
                        icon={LocateFixed}
                        aria-pressed={following}
                        className="aria-pressed:border-line-strong aria-pressed:bg-surface-3"
                        onClick={() => setFollowing(!following)}
                    >
                        {following ? words.following : words.follow}
                    </Button>
                </div>
                {(chosen.player !== null || canBan) && (
                    <div className="flex gap-2 [&>*]:flex-1">
                        {chosen.player !== null && (
                            <Link
                                to="/players/$id"
                                params={{ id: String(chosen.player) }}
                                className={buttonVariants({ size: 'sm' })}
                            >
                                <ExternalLink size={14} strokeWidth={1.5} aria-hidden="true" />
                                {words.profile}
                            </Link>
                        )}
                        {canBan && chosen.player !== null && (
                            <Button size="sm" icon={BanIcon} onClick={() => setBanning(chosen)}>
                                {words.ban}
                            </Button>
                        )}
                    </div>
                )}
            </section>
        );

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {words.title} <span className="gloss">{words.gloss}</span>
                </h1>
                <div className="flex flex-wrap items-center gap-x-5 gap-y-2 max-md:w-full">
                    <label className="flex h-9.5 items-center gap-2.5 text-ui font-medium text-ink max-md:h-11">
                        <Switch checked={names} onChange={(event) => setNames(event.target.checked)} />
                        {words.names}
                    </label>
                    <label className="flex h-9.5 items-center gap-2.5 text-ui font-medium text-ink max-md:h-11">
                        <Switch checked={trailing} onChange={(event) => setTrailing(event.target.checked)} />
                        {words.trails}
                    </label>
                    {layer.allowed && (
                        <label className="flex h-9.5 items-center gap-2.5 text-ui font-medium text-ink max-md:h-11">
                            <Switch
                                checked={layer.on}
                                onChange={(event) => layer.turn(event.target.checked)}
                            />
                            {words.entities.layer}
                        </label>
                    )}
                </div>
            </div>
            {live !== null && !live.onesync && <Notice tone="warning">{words.noOneSync}</Notice>}
            {deleted !== null && <Notice tone={deleted.tone}>{deleted.message}</Notice>}
            <div className="grid grid-cols-1 items-start gap-3 2xl:grid-cols-[minmax(0,1fr)_19rem]">
                <section className="card flex min-w-0 flex-col" aria-labelledby="map-title">
                    <div className="card-head">
                        <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                            <MapIcon size={14} strokeWidth={1.5} aria-hidden="true" />
                        </span>
                        <h2 id="map-title" className="card-title">
                            {words.players(players.length + (live?.waiting ?? 0))}
                        </h2>
                        <p className="text-meta text-ink-3">
                            {words.placed(players.length, live?.waiting ?? 0)}
                        </p>
                        <ul
                            aria-label={words.legend}
                            className="ml-auto flex flex-wrap items-center gap-x-3.5 gap-y-1.5 text-meta text-ink-2 max-xl:ml-0"
                        >
                            {TIERS.map((tier) => (
                                <li key={tier} className="flex items-center gap-1.75">
                                    <span
                                        aria-hidden="true"
                                        className={cn('inline-block flex-none', TIER_MARK[tier])}
                                    />
                                    {words.tiers[tier]}
                                    {tier !== 'unconfirmed' && (
                                        <span className="text-ink-3">
                                            {tier === 'new'
                                                ? words.tierRange.new(settings.newHours)
                                                : tier === 'regular'
                                                  ? words.tierRange.regular(
                                                        settings.newHours,
                                                        settings.veteranHours,
                                                    )
                                                  : words.tierRange.veteran(settings.veteranHours)}
                                        </span>
                                    )}
                                </li>
                            ))}
                        </ul>
                    </div>
                    <MapCanvas
                        tiles={tiles}
                        onTilesFailed={() => setUnsent(imagery)}
                        image={placedImage}
                        bounds={placedImage === null ? null : settings.bounds}
                        dots={dots}
                        trails={trailing ? [...trails].map(([id, points]) => ({ id, points })) : []}
                        selected={selected}
                        names={names}
                        focus={followFocus ?? focus}
                        frame={frame}
                        renderLayer={
                            layer.shown === null
                                ? undefined
                                : (projection) => (
                                      <EntityCanvas
                                          projection={projection}
                                          points={layer.shown?.points ?? []}
                                          cells={layer.shown?.cells ?? []}
                                          cell={layer.shown?.cell ?? null}
                                          picked={layer.picked}
                                      />
                                  )
                        }
                        onViewport={layer.allowed ? layer.watch : undefined}
                        onPress={(at) => {
                            const cell = layer.press(at);
                            if (cell !== null) {
                                setFrame({ ...cell, at: Date.now() });
                            }
                        }}
                        onSelect={(id) => {
                            setSelected(id);
                            if (id === null) {
                                setFollowing(false);
                            } else {
                                layer.pick(null);
                            }
                        }}
                        className="h-[calc(100dvh-17rem)] min-h-96 max-md:h-[62dvh] max-md:min-h-80"
                    >
                        {/* One row that wraps: the layer's bar goes under the styles where both do not fit. */}
                        <div className="pointer-events-none absolute inset-x-3 top-3 flex flex-wrap items-start gap-2 [&>*]:pointer-events-auto">
                            {isTileStyle(settings.imagery) && isTileStyle(imagery) && (
                                <div data-overlay>
                                    <Segmented
                                        label={words.style}
                                        choices={TILE_STYLES}
                                        value={imagery}
                                        words={words.styles}
                                        className="bg-surface shadow-pop"
                                        onChange={(next) => {
                                            setStyle(next);
                                            setUnsent(null);
                                            try {
                                                localStorage.setItem(STYLE_KEY, next);
                                            } catch {
                                                // Private browsing: the choice lasts for this page only.
                                            }
                                        }}
                                    />
                                </div>
                            )}
                            {layer.on && <LayerBar layer={layer} className="ml-auto" />}
                        </div>
                        {isTileStyle(imagery) && tiles === null && (
                            <output
                                data-overlay
                                className="absolute top-3 left-1/2 flex max-w-[calc(100%-1.5rem)] -translate-x-1/2 flex-wrap items-center justify-center gap-x-2 gap-y-1 rounded-box border border-line bg-surface py-1 pr-1 pl-3 text-meta text-ink-2 shadow-pop"
                            >
                                <ImageIcon
                                    size={14}
                                    strokeWidth={1.5}
                                    className="flex-none"
                                    aria-hidden="true"
                                />
                                {words.tilesUnsent}
                                <Button
                                    tone="ghost"
                                    size="sm"
                                    icon={RotateCw}
                                    onClick={() => setUnsent(null)}
                                >
                                    {words.retry}
                                </Button>
                            </output>
                        )}
                        {imagery === 'image' && placedImage === null && view.isSuccess && (
                            <div
                                data-overlay
                                className="absolute top-3 left-1/2 flex max-w-[calc(100%-1.5rem)] -translate-x-1/2 flex-wrap items-center justify-center gap-x-2 gap-y-1 rounded-box border border-line bg-surface px-3 py-1.5 text-meta text-ink-2 shadow-pop"
                            >
                                <ImageIcon
                                    size={14}
                                    strokeWidth={1.5}
                                    className="flex-none"
                                    aria-hidden="true"
                                />
                                {image === null ? words.noImage : words.notCalibrated}
                                {me.isOwner && (
                                    <Link
                                        to="/settings/$category"
                                        params={{ category: 'map' }}
                                        className="font-medium text-accent-ink underline-offset-2 hover:underline"
                                    >
                                        {image === null ? words.addImage : words.placeImage}
                                    </Link>
                                )}
                            </div>
                        )}
                        {players.length === 0 && live !== null && (
                            <p className="pointer-events-none absolute top-1/2 left-1/2 max-w-[calc(100%-1.5rem)] -translate-1/2 rounded-box border border-line bg-surface px-3 py-1.5 text-center text-ui text-ink-2 shadow-pop">
                                {running ? words.waitingFirst : words.stopped}
                            </p>
                        )}
                        {!beside && card}
                    </MapCanvas>
                    <div className="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3 max-md:pl-4">
                        <span className="mr-auto">{words.every(settings.everySeconds)}</span>
                        {layer.live !== null && layer.live.everyMs > 0 && (
                            <span>{words.entities.every(Math.round(layer.live.everyMs / 1000))}</span>
                        )}
                        {tiles !== null && <span>{words.credit}</span>}
                        {live?.at != null && (
                            <span>
                                {words.updated(
                                    format.duration(Math.max(0, now - Date.parse(live.at)) / 1000),
                                )}
                            </span>
                        )}
                    </div>
                </section>
                <aside className="card flex min-w-0 flex-col" aria-labelledby={listId}>
                    {beside && card}
                    <div className="flex flex-col gap-3 border-b border-line p-4">
                        <h2 id={listId} className="card-title">
                            {words.list}
                        </h2>
                        <form
                            className="flex flex-col gap-1"
                            onSubmit={(event) => {
                                event.preventDefault();
                                search();
                            }}
                        >
                            <div className="relative">
                                <input
                                    type="search"
                                    aria-label={words.find}
                                    placeholder={words.findHint}
                                    className={cn(inputClass, 'w-full pl-8.5')}
                                    value={find}
                                    aria-invalid={missed || undefined}
                                    onChange={(event) => {
                                        setFind(event.target.value);
                                        setMissed(false);
                                    }}
                                />
                                <Search
                                    size={16}
                                    strokeWidth={1.5}
                                    className="pointer-events-none absolute top-1/2 left-2.75 -translate-y-1/2 text-ink-3"
                                    aria-hidden="true"
                                />
                            </div>
                            {missed && <FieldError>{words.noMatch}</FieldError>}
                        </form>
                    </div>
                    {listed.length === 0 ? (
                        <p className="p-4 text-small text-ink-3">
                            {players.length === 0 ? words.nobody : words.noMatch}
                        </p>
                    ) : (
                        <ul className="grid max-h-[calc(100dvh-24rem)] min-h-24 grid-cols-1 overflow-y-auto overscroll-contain p-1.5 scrollbar-thin max-2xl:max-h-72 max-2xl:sm:grid-cols-2 max-2xl:lg:grid-cols-3">
                            {listed.map((player) => {
                                const tier = tierOf(player.player === null ? null : player.totalMs, settings);
                                const ping = pings.get(player.netId);
                                return (
                                    <li key={player.netId}>
                                        <button
                                            type="button"
                                            aria-pressed={player.netId === selected}
                                            className="flex min-h-9 w-full items-center gap-2.5 rounded-control px-2.5 py-1.5 text-left hover:bg-surface-2 aria-pressed:bg-surface-3 max-md:min-h-11"
                                            onClick={() => show(player.netId)}
                                        >
                                            <span
                                                aria-hidden="true"
                                                className={cn('inline-block flex-none', TIER_MARK[tier])}
                                            />
                                            <span className="min-w-0 flex-1 truncate text-ui text-ink">
                                                {nameOf(player)}
                                            </span>
                                            <span className="sr-only">{words.tiers[tier]}</span>
                                            <span className="flex-none font-mono text-meta text-ink-3 tabular-nums">
                                                {ping == null
                                                    ? `#${player.netId}`
                                                    : format.milliseconds(ping)}
                                            </span>
                                        </button>
                                    </li>
                                );
                            })}
                        </ul>
                    )}
                    <p className="border-t border-line p-4 text-small text-ink-3">{words.listHint}</p>
                </aside>
            </div>
            {deleting !== null && layer.live?.run != null && layer.limits !== null && (
                <DeleteDialog
                    items={[deleting]}
                    run={layer.live.run}
                    limits={layer.limits}
                    onClose={() => setDeleting(null)}
                    onDone={(results) => {
                        setDeleted(deletionNotice(t, results));
                        setDeleting(null);
                        layer.pick(null);
                    }}
                />
            )}
            {banning !== null && banning.player !== null && (
                <BanDialog
                    target={{
                        player: {
                            id: banning.player,
                            name: banning.name ?? words.unnamed(banning.netId),
                            license: session?.license ?? '',
                            online: true,
                        },
                    }}
                    onClose={() => setBanning(null)}
                    onBanned={() => setBanning(null)}
                />
            )}
        </div>
    );
}

function Fact({ term, children }: { term: string; children: ReactNode }) {
    return (
        <div className="flex min-w-0 flex-col">
            <dt className="text-meta text-ink-3">{term}</dt>
            <dd className="font-mono text-small text-ink [overflow-wrap:anywhere]">{children}</dd>
        </div>
    );
}
