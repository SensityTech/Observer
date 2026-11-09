import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Check, ChevronDown, Image as ImageIcon, Trash2 } from 'lucide-react';
import { useState, type MouseEvent } from 'react';

import type { MapBounds } from '../api/generated/MapBounds';
import type { MapImage } from '../api/generated/MapImage';
import type { MapPlayer } from '../api/generated/MapPlayer';
import type { MapSettings } from '../api/generated/MapSettings';
import type { MapView } from '../api/generated/MapView';
import { Permission } from '../api/generated/Permission';
import { CADENCES, mapQuery, removeMapImage, saveMapSettings, setMapImage, useMapImage } from '../api/map';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Chip } from '../components/Chip';
import { FieldError, inputClass, TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useTopic } from '../live/context';
import { boundsFrom, type Spot } from '../map/geometry';
import { useVisit } from '../players/words';
import { useSchedule } from '../schedule/context';
import { SettingsForm, useSection } from './Form';
import { CategoryHead, SettingRow, SettingsGroup } from './Row';

type Outcome = { tone: 'success' | 'danger'; message: string };
type Way = 'players' | 'edges';
type Edges = Record<keyof MapBounds, string>;
const EDGES: (keyof MapBounds)[] = ['west', 'east', 'south', 'north'];
const WAYS: Way[] = ['players', 'edges'];

/** The live map's image, where it sits in the game, its colours and how often positions come. */
export function LiveMapSettings() {
    const { t } = useI18n();
    const me = useMe();
    const view = useQuery(mapQuery);
    return (
        <>
            <CategoryHead
                category="map"
                aside={
                    allows(me, Permission.PlayersMap) ? (
                        <Link
                            to="/map"
                            className="inline-flex items-center gap-1.5 text-ui font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                        >
                            {t.settings.map.openMap}
                            <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                        </Link>
                    ) : undefined
                }
            />
            {view.isPending && (
                <section className="card">
                    <div className="card-body flex flex-col gap-3" aria-busy="true">
                        <span className="sr-only">{t.common.loading}</span>
                        <span className="skeleton w-1/3" />
                        <span className="skeleton w-1/2" />
                    </div>
                </section>
            )}
            {view.isError && (
                <Notice tone="danger">
                    {t.settings.map.loadFailed} {errorMessage(t, view.error)}
                </Notice>
            )}
            {view.isSuccess && (
                <SettingsForm>
                    <MapForm view={view.data} />
                </SettingsForm>
            )}
        </>
    );
}

const edgesOf = (bounds: MapBounds | null): Edges =>
    bounds === null
        ? { west: '', east: '', south: '', north: '' }
        : {
              west: String(bounds.west),
              east: String(bounds.east),
              south: String(bounds.south),
              north: String(bounds.north),
          };

/** A number as typed; an empty field is none. */
const numberOf = (text: string) => (text.trim() === '' ? Number.NaN : Number(text));

/** The edges as typed, or null while one is not a number or they enclose no area. */
const boundsOf = (edges: Edges): MapBounds | null => {
    const bounds = {
        west: numberOf(edges.west),
        east: numberOf(edges.east),
        south: numberOf(edges.south),
        north: numberOf(edges.north),
    };
    return Object.values(bounds).every(Number.isFinite) &&
        bounds.east > bounds.west &&
        bounds.north > bounds.south
        ? bounds
        : null;
};

function MapForm({ view }: { view: MapView }) {
    const { t } = useI18n();
    const words = t.settings.map;
    const me = useMe();
    const client = useQueryClient();
    const canManage = allows(me, Permission.SettingsManage);
    const stored = view.settings;
    const [edges, setEdges] = useState<Edges>(edgesOf(stored.bounds));
    const [hours, setHours] = useState({
        newHours: String(stored.newHours),
        veteranHours: String(stored.veteranHours),
    });
    const [every, setEvery] = useState(stored.everySeconds);
    const [tried, setTried] = useState(false);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const typedEdges = EDGES.some((edge) => edges[edge].trim() !== '');
    const bounds = typedEdges ? boundsOf(edges) : null;
    const edgesWrong = typedEdges && bounds === null;
    const newHours = Number(hours.newHours);
    const veteranHours = Number(hours.veteranHours);
    const tiersWrong =
        !Number.isInteger(newHours) ||
        !Number.isInteger(veteranHours) ||
        newHours < 1 ||
        veteranHours <= newHours;
    const changed = [
        ...(JSON.stringify(bounds) !== JSON.stringify(stored.bounds) ? (['mapBounds'] as const) : []),
        ...(newHours !== stored.newHours || veteranHours !== stored.veteranHours
            ? (['mapTiers'] as const)
            : []),
        ...(every !== stored.everySeconds ? (['mapCadence'] as const) : []),
    ];
    const save = useMutation({
        mutationFn: () => saveMapSettings({ bounds, newHours, veteranHours, everySeconds: every }),
        onMutate: () => setOutcome(null),
        onSuccess: (saved) => {
            client.setQueryData(mapQuery.queryKey, { ...view, settings: saved });
            setEdges(edgesOf(saved.bounds));
            setTried(false);
            setOutcome({ tone: 'success', message: words.saved });
        },
        onError: (error) => setOutcome({ tone: 'danger', message: errorMessage(t, error) }),
    });
    const reset = (settings: MapSettings) => {
        setEdges(edgesOf(settings.bounds));
        setHours({ newHours: String(settings.newHours), veteranHours: String(settings.veteranHours) });
        setEvery(settings.everySeconds);
    };
    useSection({
        changed: canManage ? changed : [],
        pending: save.isPending,
        save: () => {
            setTried(true);
            if (!edgesWrong && !tiersWrong) {
                save.mutate();
            }
        },
        discard: () => {
            reset(stored);
            setTried(false);
            setOutcome({ tone: 'success', message: words.discarded });
        },
    });

    return (
        <div className="flex min-w-0 flex-col gap-4">
            {!canManage && <Notice>{words.readOnly}</Notice>}
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            <SettingsGroup title={t.settings.categories.map.name}>
                <SettingRow id="mapImage">{() => <ImageRow image={view.image} />}</SettingRow>
                <fieldset disabled={!canManage} className="contents">
                    <SettingRow id="mapBounds" changed={canManage && changed.includes('mapBounds')}>
                        {(ids) => (
                            <Placement
                                labelledBy={ids.label}
                                image={view.image}
                                edges={edges}
                                placed={bounds !== null}
                                wrong={tried && edgesWrong}
                                onEdges={setEdges}
                            />
                        )}
                    </SettingRow>
                    <SettingRow id="mapTiers" changed={canManage && changed.includes('mapTiers')}>
                        {(ids) => (
                            <fieldset
                                aria-labelledby={ids.label}
                                className="m-0 flex min-w-0 flex-col gap-2 border-0 p-0"
                            >
                                <div className="flex flex-wrap items-end gap-3">
                                    {(['newHours', 'veteranHours'] as const).map((key) => (
                                        <label
                                            key={key}
                                            className="flex flex-col gap-1.5 text-ui font-medium text-ink"
                                        >
                                            {key === 'newHours' ? words.newUnder : words.veteranFrom}
                                            <span className="flex items-center gap-2">
                                                <input
                                                    type="number"
                                                    inputMode="numeric"
                                                    min={1}
                                                    className={cn(inputClass, 'w-28')}
                                                    aria-invalid={(tried && tiersWrong) || undefined}
                                                    value={hours[key]}
                                                    onChange={(event) =>
                                                        setHours({ ...hours, [key]: event.target.value })
                                                    }
                                                />
                                                <span className="font-normal text-ink-3">{words.hours}</span>
                                            </span>
                                        </label>
                                    ))}
                                </div>
                                {tried && tiersWrong && <FieldError>{words.tiersInvalid}</FieldError>}
                            </fieldset>
                        )}
                    </SettingRow>
                    <SettingRow id="mapCadence" changed={canManage && changed.includes('mapCadence')}>
                        {() => (
                            <Segmented
                                label={t.settings.items.mapCadence.label}
                                choices={CADENCES.map(String)}
                                value={String(every)}
                                words={Object.fromEntries(
                                    CADENCES.map((value) => [String(value), words.seconds(value)]),
                                )}
                                onChange={(value) => setEvery(Number(value))}
                            />
                        )}
                    </SettingRow>
                </fieldset>
            </SettingsGroup>
        </div>
    );
}

/** The image kept, and the owner's way to give another from a path on the host. */
function ImageRow({ image }: { image: MapImage | null }) {
    const { t, format } = useI18n();
    const words = t.settings.map;
    const me = useMe();
    const client = useQueryClient();
    const zone = useSchedule().zone ?? 'UTC';
    const visit = useVisit(zone);
    const [path, setPath] = useState('');
    const [done, setDone] = useState<string | null>(null);
    const add = useMutation({
        mutationFn: () => setMapImage(path.trim()),
        onSuccess: async () => {
            setPath('');
            setDone(words.added);
            await client.invalidateQueries(mapQuery);
        },
    });
    const remove = useMutation({
        mutationFn: removeMapImage,
        onSuccess: async () => {
            setDone(words.removed);
            await client.invalidateQueries(mapQuery);
        },
    });
    return (
        <div className="flex flex-col gap-3">
            <p className="flex items-start gap-2 text-ui text-ink-2">
                <ImageIcon
                    size={16}
                    strokeWidth={1.5}
                    className="mt-0.5 flex-none text-ink-3"
                    aria-hidden="true"
                />
                {image === null
                    ? words.none
                    : words.image(
                          image.width,
                          image.height,
                          format.bytes(image.bytes),
                          image.addedBy,
                          visit(image.addedAt),
                      )}
            </p>
            {me.isOwner && (
                <div className="flex flex-col gap-2">
                    <TextField
                        label={words.path}
                        value={path}
                        spellCheck={false}
                        autoCapitalize="none"
                        autoComplete="off"
                        inputClassName="font-mono text-ui"
                        onChange={(event) => setPath(event.target.value)}
                        beside={
                            <Button
                                icon={Check}
                                pending={add.isPending}
                                disabled={path.trim() === ''}
                                onClick={() => {
                                    setDone(null);
                                    add.mutate();
                                }}
                            >
                                {words.use}
                            </Button>
                        }
                    />
                    {image !== null && (
                        <Button
                            tone="ghost"
                            icon={Trash2}
                            className="self-start"
                            pending={remove.isPending}
                            onClick={() => {
                                setDone(null);
                                remove.mutate();
                            }}
                        >
                            {words.remove}
                        </Button>
                    )}
                </div>
            )}
            {done !== null && <Notice tone="success">{done}</Notice>}
            {add.isError && <Notice tone="danger">{errorMessage(t, add.error)}</Notice>}
            {remove.isError && <Notice tone="danger">{errorMessage(t, remove.error)}</Notice>}
        </div>
    );
}

type Mark = {
    player: number | null;
    world: { x: number; y: number } | null;
    pixel: { x: number; y: number } | null;
};
const EMPTY: Mark = { player: null, world: null, pixel: null };

/** Where the image sits: two spots where players stand, or the four edges typed. */
function Placement({
    labelledBy,
    image,
    edges,
    placed,
    wrong,
    onEdges,
}: {
    labelledBy: string;
    image: MapImage | null;
    edges: Edges;
    placed: boolean;
    wrong: boolean;
    onEdges: (edges: Edges) => void;
}) {
    const { t } = useI18n();
    const words = t.settings.map;
    const [way, setWay] = useState<Way>(image === null ? 'edges' : 'players');
    return (
        <fieldset aria-labelledby={labelledBy} className="m-0 flex min-w-0 flex-col gap-3 border-0 p-0">
            <div className="flex flex-wrap items-center gap-3">
                <Segmented
                    label={t.settings.items.mapBounds.label}
                    choices={WAYS}
                    value={way}
                    words={words.ways}
                    onChange={setWay}
                />
                <Chip tone={placed ? 'success' : 'neutral'}>
                    {placed ? words.calibrated : words.notCalibrated}
                </Chip>
            </div>
            {way === 'players' && image !== null && (
                <BySpots
                    image={image}
                    onPlaced={(bounds) => {
                        onEdges(edgesOf(bounds));
                        setWay('edges');
                    }}
                />
            )}
            {way === 'edges' && (
                <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
                    {EDGES.map((edge) => (
                        <label
                            key={edge}
                            className="flex min-w-0 flex-col gap-1.5 text-ui font-medium text-ink"
                        >
                            {words.edges[edge]}
                            <input
                                type="number"
                                inputMode="decimal"
                                className={cn(inputClass, 'font-mono')}
                                aria-invalid={wrong || undefined}
                                value={edges[edge]}
                                onChange={(event) => onEdges({ ...edges, [edge]: event.target.value })}
                            />
                        </label>
                    ))}
                </div>
            )}
            {wrong && <FieldError>{words.edgesInvalid}</FieldError>}
        </fieldset>
    );
}

/** Two spots: a connected player picked, then clicked where they stand on the image. */
function BySpots({ image, onPlaced }: { image: MapImage; onPlaced: (bounds: MapBounds) => void }) {
    const { t } = useI18n();
    const words = t.settings.map;
    const [players, setPlayers] = useState<MapPlayer[]>([]);
    // Following the topic is what has the server send positions while the owner places the image.
    useTopic('server.map', {
        snapshot: (live) => setPlayers(live.players),
        event: (live) => setPlayers(live.players),
        cleared: () => setPlayers([]),
    });
    const url = useMapImage(image.version);
    const [marks, setMarks] = useState<[Mark, Mark]>([EMPTY, EMPTY]);
    const [active, setActive] = useState<0 | 1>(0);
    const [tooClose, setTooClose] = useState(false);
    const nameOf = (player: MapPlayer) => player.label?.name ?? player.name ?? t.map.unnamed(player.netId);
    const update = (index: 0 | 1, mark: Mark) => {
        const next: [Mark, Mark] = [...marks];
        next[index] = mark;
        setMarks(next);
        setTooClose(false);
    };
    const click = (event: MouseEvent<HTMLButtonElement>) => {
        const rect = event.currentTarget.getBoundingClientRect();
        const pixel = {
            x: Math.round(((event.clientX - rect.left) / rect.width) * image.width),
            y: Math.round(((event.clientY - rect.top) / rect.height) * image.height),
        };
        update(active, { ...marks[active], pixel });
        if (active === 0) {
            setActive(1);
        }
    };
    const ready = marks.every((mark) => mark.world !== null && mark.pixel !== null);
    return (
        <div className="flex flex-col gap-3">
            {players.length === 0 && <Notice>{words.noOneOnline}</Notice>}
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                {([0, 1] as const).map((index) => {
                    const mark = marks[index];
                    return (
                        <fieldset
                            key={index}
                            className={cn(
                                'flex min-w-0 flex-col gap-2 rounded-box border p-3',
                                active === index ? 'border-accent bg-accent-soft' : 'border-line',
                            )}
                        >
                            <legend className="sr-only">{words.spot(index + 1)}</legend>
                            <span className="text-ui font-medium text-ink">{words.spot(index + 1)}</span>
                            <div className="relative">
                                <select
                                    aria-label={words.pick}
                                    className={cn(inputClass, 'appearance-none pr-9')}
                                    value={mark.player ?? ''}
                                    onChange={(event) => {
                                        const player = players.find(
                                            (each) => String(each.netId) === event.target.value,
                                        );
                                        update(index, {
                                            ...mark,
                                            player: player?.netId ?? null,
                                            world: player === undefined ? null : { x: player.x, y: player.y },
                                        });
                                        setActive(index);
                                    }}
                                >
                                    <option value="">{words.pickNobody}</option>
                                    {players.map((player) => (
                                        <option key={player.netId} value={player.netId}>
                                            {words.standsAt(
                                                nameOf(player),
                                                player.x.toFixed(1),
                                                player.y.toFixed(1),
                                            )}
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
                            <span className="font-mono text-small text-ink-3">
                                {mark.pixel === null
                                    ? words.notClicked
                                    : words.onImage(String(mark.pixel.x), String(mark.pixel.y))}
                            </span>
                        </fieldset>
                    );
                })}
            </div>
            <p className="text-small text-ink-3">{words.clickWhere}</p>
            {url !== null && (
                <button
                    type="button"
                    aria-label={words.clickWhere}
                    className="relative block w-full cursor-crosshair overflow-hidden rounded-box border border-line bg-surface-2"
                    style={{ aspectRatio: `${image.width} / ${image.height}`, maxHeight: '28rem' }}
                    onClick={click}
                >
                    <img src={url} alt="" draggable={false} className="block size-full object-fill" />
                    {(['first', 'second'] as const).map((name, index) => {
                        const mark = marks[index];
                        return (
                            mark?.pixel != null && (
                                <span
                                    key={name}
                                    aria-hidden="true"
                                    className="absolute grid size-5 -translate-1/2 place-items-center rounded-full border-2 border-surface bg-accent font-mono text-micro font-semibold text-on-accent"
                                    style={{
                                        left: `${(mark.pixel.x / image.width) * 100}%`,
                                        top: `${(mark.pixel.y / image.height) * 100}%`,
                                    }}
                                >
                                    {index + 1}
                                </span>
                            )
                        );
                    })}
                </button>
            )}
            {tooClose && <FieldError>{words.tooClose}</FieldError>}
            <Button
                className="self-start"
                disabled={!ready}
                onClick={() => {
                    const [first, second] = marks;
                    if (
                        first.world === null ||
                        first.pixel === null ||
                        second.world === null ||
                        second.pixel === null
                    ) {
                        return;
                    }
                    const spots: [Spot, Spot] = [
                        { world: first.world, pixel: first.pixel },
                        { world: second.world, pixel: second.pixel },
                    ];
                    const bounds = boundsFrom(spots[0], spots[1], image.width, image.height);
                    if (bounds === null) {
                        setTooClose(true);
                        return;
                    }
                    onPlaced(bounds);
                }}
            >
                {words.apply}
            </Button>
        </div>
    );
}
