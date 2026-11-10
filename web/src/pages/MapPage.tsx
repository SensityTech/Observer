import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import {
    Ban as BanIcon,
    ExternalLink,
    Image as ImageIcon,
    Map as MapIcon,
    RotateCw,
    Search,
    X,
} from 'lucide-react';
import { useMemo, useState, type ReactNode } from 'react';

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
import { Switch } from '../components/Switch';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useNow } from '../lib/useNow';
import { useTopic } from '../live/context';
import { tierOf, type Tier } from '../map/geometry';
import { MapCanvas, TIER_MARK, type Dot } from '../map/MapCanvas';
import { isTileStyle } from '../map/tiles';
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

export function MapPage() {
    const { t, format } = useI18n();
    const words = t.map;
    const me = useMe();
    const view = useQuery(mapQuery);
    const online = useQuery(onlineQuery);
    const status = useServerStatus().status;
    const [live, setLive] = useState<MapLive | null>(null);
    useTopic('server.map', {
        snapshot: setLive,
        event: setLive,
        refused: () => setLive(null),
        cleared: () => setLive(null),
    });
    const settings = view.data?.settings ?? DEFAULTS;
    const image = view.data?.image ?? null;
    const imageUrl = useMapImage(image?.version ?? null);
    const [selected, setSelected] = useState<number | null>(null);
    const [names, setNames] = useState(false);
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
    const { imagery } = settings;
    const tiles = isTileStyle(imagery) && unsent !== imagery ? imagery : null;
    const placedImage =
        imagery === 'image' && image !== null && imageUrl !== null && settings.bounds !== null
            ? { url: imageUrl, width: image.width, height: image.height }
            : null;
    const running = status !== null && ['starting', 'online', 'degraded', 'hung'].includes(status.state);
    const canBan = allows(me, Permission.PlayersBan);

    const search = () => {
        const wanted = find.trim().toLowerCase();
        if (wanted === '') {
            return;
        }
        const found = players.find(
            (player) =>
                nameOf(player).toLowerCase().includes(wanted) ||
                (player.name ?? '').toLowerCase().includes(wanted) ||
                String(player.netId) === wanted,
        );
        setMissed(found === undefined);
        if (found !== undefined) {
            setSelected(found.netId);
            setFocus({ id: found.netId, at: Date.now() });
        }
    };

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {words.title} <span className="gloss">{words.gloss}</span>
                </h1>
                <div className="flex flex-wrap items-start gap-3 max-md:w-full">
                    <form
                        className="flex min-w-0 flex-col gap-1 max-md:flex-1"
                        onSubmit={(event) => {
                            event.preventDefault();
                            search();
                        }}
                    >
                        <div className="relative">
                            <input
                                type="search"
                                aria-label={words.find}
                                placeholder={words.find}
                                className={cn(inputClass, 'w-64 pl-8.5 max-md:w-full')}
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
                    <label className="flex h-9.5 items-center gap-2.5 text-ui font-medium text-ink max-md:h-11">
                        <Switch checked={names} onChange={(event) => setNames(event.target.checked)} />
                        {words.names}
                    </label>
                </div>
            </div>
            {live !== null && !live.onesync && <Notice tone="warning">{words.noOneSync}</Notice>}
            <section className="card flex flex-col" aria-labelledby="map-title">
                <div className="card-head">
                    <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                        <MapIcon size={14} strokeWidth={1.5} aria-hidden="true" />
                    </span>
                    <h2 id="map-title" className="card-title">
                        {words.players(players.length + (live?.waiting ?? 0))}
                    </h2>
                    <p className="text-meta text-ink-3">{words.placed(players.length, live?.waiting ?? 0)}</p>
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
                    selected={selected}
                    names={names}
                    focus={focus}
                    onSelect={setSelected}
                    className="h-[calc(100dvh-17rem)] min-h-96 max-md:h-[62dvh] max-md:min-h-80"
                >
                    {isTileStyle(imagery) && tiles === null && (
                        <output
                            data-overlay
                            className="absolute top-3 left-1/2 flex max-w-[calc(100%-1.5rem)] -translate-x-1/2 flex-wrap items-center justify-center gap-x-2 gap-y-1 rounded-box border border-line bg-surface py-1 pr-1 pl-3 text-meta text-ink-2 shadow-pop"
                        >
                            <ImageIcon size={14} strokeWidth={1.5} className="flex-none" aria-hidden="true" />
                            {words.tilesUnsent}
                            <Button tone="ghost" size="sm" icon={RotateCw} onClick={() => setUnsent(null)}>
                                {words.retry}
                            </Button>
                        </output>
                    )}
                    {imagery === 'image' && placedImage === null && view.isSuccess && (
                        <div
                            data-overlay
                            className="absolute top-3 left-1/2 flex max-w-[calc(100%-1.5rem)] -translate-x-1/2 flex-wrap items-center justify-center gap-x-2 gap-y-1 rounded-box border border-line bg-surface px-3 py-1.5 text-meta text-ink-2 shadow-pop"
                        >
                            <ImageIcon size={14} strokeWidth={1.5} className="flex-none" aria-hidden="true" />
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
                    {chosen !== null && (
                        <section
                            data-overlay
                            aria-label={words.selected}
                            className="absolute top-14 right-3 flex w-72 flex-col gap-3 rounded-box border border-line-strong bg-surface p-3.5 shadow-pop max-md:inset-x-2 max-md:top-auto max-md:bottom-2 max-md:w-auto"
                        >
                            <div className="flex items-start gap-2.5">
                                <span
                                    aria-hidden="true"
                                    className={cn(
                                        'mt-1.5 inline-block flex-none',
                                        TIER_MARK[
                                            tierOf(chosen.player === null ? null : chosen.totalMs, settings)
                                        ],
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
                                    onClick={() => setSelected(null)}
                                >
                                    <X size={14} strokeWidth={1.5} aria-hidden="true" />
                                </Button>
                            </div>
                            <div className="flex flex-wrap gap-1.5">
                                <Chip>
                                    {
                                        words.tiers[
                                            tierOf(chosen.player === null ? null : chosen.totalMs, settings)
                                        ]
                                    }
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
                                    {session?.ping == null
                                        ? words.unknownPing
                                        : format.milliseconds(session.ping)}
                                </Fact>
                                <Fact term={words.position}>
                                    {`${chosen.x.toFixed(1)}, ${chosen.y.toFixed(1)}, ${chosen.z.toFixed(1)}`}
                                </Fact>
                            </dl>
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
                    )}
                </MapCanvas>
                <div className="flex flex-wrap items-center gap-x-4 gap-y-1 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3 max-md:pl-4">
                    <span className="mr-auto">{words.every(settings.everySeconds)}</span>
                    {tiles !== null && <span>{words.credit}</span>}
                    {live?.at != null && (
                        <span>
                            {words.updated(format.duration(Math.max(0, now - Date.parse(live.at)) / 1000))}
                        </span>
                    )}
                </div>
            </section>
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
