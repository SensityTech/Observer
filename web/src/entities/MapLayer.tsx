import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { List, Trash2, X } from 'lucide-react';
import { useCallback, useLayoutEffect, useRef, useState, type ReactNode } from 'react';

import { entitiesMapQuery, entityQuery, NO_FILTERS } from '../api/entities';
import type { EntityCell } from '../api/generated/EntityCell';
import type { EntityItem } from '../api/generated/EntityItem';
import type { EntityKind } from '../api/generated/EntityKind';
import type { EntityPoint } from '../api/generated/EntityPoint';
import type { KindCounts } from '../api/generated/KindCounts';
import type { MapBounds } from '../api/generated/MapBounds';
import { Button, buttonVariants } from '../components/Button';
import { Chip } from '../components/Chip';
import { CopyValue } from '../components/CopyValue';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import type { Press, Projection, Viewport } from '../map/MapCanvas';
import { boundsFor, cellAt, cellBounds, cellFor, nearest, shortCount, strength } from './layer';
import { useEntitiesLive } from './live';
import { EntityMark, entityName, kindWord } from './marks';
import { blocker, modelHash, scripted } from './model';
import { KINDS } from './search';

/** The side of the cells asked for until the map said how far it is zoomed. */
const FIRST_CELL = 200;
/** A cell this wide on screen has room for its count. */
const COUNTED_FROM = 30;
/** The numbers of the kinds on the wire, in the order the map draws them: what matters most last. */
const DRAWN: readonly [number, EntityKind][] = [
    [3, 'object'],
    [1, 'ped'],
    [2, 'vehicle'],
];
const COLOURS: Record<EntityKind, string> = {
    vehicle: '--term-cyan',
    ped: '--term-magenta',
    object: '--ink-3',
};
const COUNT_KEYS: Record<EntityKind, keyof KindCounts> = {
    vehicle: 'vehicles',
    ped: 'peds',
    object: 'objects',
};

const sameBounds = (left: MapBounds, right: MapBounds) =>
    left.west === right.west &&
    left.east === right.east &&
    left.south === right.south &&
    left.north === right.north;

/**
 * The entities' layer of the live map: on or off, the kinds shown, the entities of the part of the
 * map in view, and the one picked. The server lists its entities only while the layer is on.
 */
export const useEntityLayer = (allowed: boolean, wanted: number | undefined) => {
    const [on, setOn] = useState(allowed && wanted !== undefined);
    const [kinds, setKinds] = useState<readonly EntityKind[]>(KINDS);
    const [asked, setAsked] = useState<{ bounds: MapBounds; cell: number } | null>(null);
    const [picked, setPicked] = useState<number | null>(wanted ?? null);
    const active = allowed && on;
    const live = useEntitiesLive(active);
    const map = useQuery({
        ...entitiesMapQuery(
            { ...NO_FILTERS, kinds: kinds.length === KINDS.length ? [] : [...kinds] },
            asked?.cell ?? FIRST_CELL,
            asked?.bounds ?? null,
        ),
        enabled: active && asked !== null && kinds.length > 0,
        placeholderData: keepPreviousData,
    });
    const detail = useQuery({ ...entityQuery(picked ?? 0), enabled: active && picked !== null });
    const shown = active && kinds.length > 0 ? (map.data ?? null) : null;
    const item = picked === null ? null : (detail.data?.items.find((each) => each.netId === picked) ?? null);

    const watch = useCallback((viewport: Viewport) => {
        const bounds = boundsFor(viewport.bounds);
        const cell = cellFor(viewport.unitsPerPixel);
        setAsked((current) =>
            current !== null && current.cell === cell && sameBounds(current.bounds, bounds)
                ? current
                : { bounds, cell },
        );
    }, []);

    /** A press on the map: picks the entity under it, or gives the cell under it to zoom into. */
    const press = (at: Press): MapBounds | null => {
        if (shown === null) {
            return null;
        }
        const point = nearest(shown.points, at);
        setPicked(point === null ? null : point[0]);
        if (point !== null || shown.cell === null) {
            return null;
        }
        const cell = cellAt(shown.cells, shown.cell, at.x, at.y);
        return cell === null ? null : cellBounds(cell, shown.cell);
    };

    return {
        allowed,
        on: active,
        turn: (next: boolean) => {
            setOn(next);
            if (!next) {
                setPicked(null);
            }
        },
        kinds,
        toggle: (kind: EntityKind) =>
            setKinds(kinds.includes(kind) ? kinds.filter((each) => each !== kind) : [...kinds, kind]),
        live,
        /** The entities to draw, or null while the layer is off or shows no kind. */
        shown,
        failed: active && map.isError ? map.error : null,
        picked: active ? picked : null,
        pick: setPicked,
        /** The picked entity as the list has it; null while it is read, and once it is gone. */
        item,
        itemRead: detail.isSuccess,
        limits: detail.data?.limits ?? null,
        ownersShown: detail.data?.ownersShown ?? false,
        watch,
        press,
    };
};

export type EntityLayer = ReturnType<typeof useEntityLayer>;

/**
 * The entities, drawn on one canvas under the players: thousands of marks move with the map at each
 * frame. The list page is where a keyboard and a screen reader reach them.
 */
export function EntityCanvas({
    projection,
    points,
    cells,
    cell,
    picked,
}: {
    projection: Projection;
    points: readonly EntityPoint[];
    cells: readonly EntityCell[];
    /** The side of a cell in game units, when the map counts instead of drawing each entity. */
    cell: number | null;
    picked: number | null;
}) {
    const canvas = useRef<HTMLCanvasElement>(null);
    useLayoutEffect(() => {
        const element = canvas.current;
        const context = element?.getContext('2d') ?? null;
        if (element === null || context === null) {
            return;
        }
        const ratio = window.devicePixelRatio || 1;
        const width = Math.round(projection.width * ratio);
        const height = Math.round(projection.height * ratio);
        if (element.width !== width || element.height !== height) {
            element.width = width;
            element.height = height;
        }
        const style = getComputedStyle(element);
        const token = (name: string) => style.getPropertyValue(name).trim();
        const colours = Object.fromEntries(KINDS.map((kind) => [kind, token(COLOURS[kind])])) as Record<
            EntityKind,
            string
        >;
        const ground = token('--bg-subtle');
        const ink = token('--ink');
        context.setTransform(ratio, 0, 0, ratio, 0, 0);
        context.clearRect(0, 0, projection.width, projection.height);

        if (cell !== null) {
            const most = cells.reduce((highest, each) => Math.max(highest, each[2]), 0);
            context.font = `600 11px ${token('--font-mono') || 'monospace'}`;
            context.textAlign = 'center';
            context.textBaseline = 'middle';
            for (const [column, row, count, kind] of cells) {
                const corner = projection.place(column * cell, (row + 1) * cell);
                const opposite = projection.place((column + 1) * cell, row * cell);
                const side = opposite.left - corner.left;
                if (
                    opposite.left < 0 ||
                    opposite.top < 0 ||
                    corner.left > projection.width ||
                    corner.top > projection.height
                ) {
                    continue;
                }
                const name = DRAWN.find(([code]) => code === kind)?.[1] ?? 'object';
                // Never so strong that the map under a cell stops showing.
                context.globalAlpha = 0.16 + 0.44 * strength(count, most);
                context.fillStyle = colours[name];
                context.fillRect(
                    corner.left + 0.5,
                    corner.top + 0.5,
                    side - 1,
                    opposite.top - corner.top - 1,
                );
                if (side >= COUNTED_FROM) {
                    context.globalAlpha = 1;
                    context.lineWidth = 3;
                    context.strokeStyle = ground;
                    context.fillStyle = ink;
                    const centre = [corner.left + side / 2, (corner.top + opposite.top) / 2] as const;
                    context.strokeText(shortCount(count), ...centre);
                    context.fillText(shortCount(count), ...centre);
                }
            }
            context.globalAlpha = 1;
        }

        for (const [code, kind] of DRAWN) {
            for (const point of points) {
                if (point[2] !== code) {
                    continue;
                }
                const { left, top } = projection.place(point[3], point[4]);
                if (left < -8 || top < -8 || left > projection.width + 8 || top > projection.height + 8) {
                    continue;
                }
                // Each mark over a ring of the ground's colour, so that two that touch stay two.
                for (const [grown, fill] of [
                    [1.5, ground],
                    [0, colours[kind]],
                ] as const) {
                    context.fillStyle = fill;
                    context.beginPath();
                    if (kind === 'vehicle') {
                        context.arc(left, top, 5 + grown, 0, 2 * Math.PI);
                    } else if (kind === 'ped') {
                        const reach = 6 + grown * 1.4;
                        context.moveTo(left, top - reach);
                        context.lineTo(left + reach, top);
                        context.lineTo(left, top + reach);
                        context.lineTo(left - reach, top);
                        context.closePath();
                    } else {
                        const half = 4 + grown;
                        context.rect(left - half, top - half, 2 * half, 2 * half);
                    }
                    context.fill();
                }
                if (point[0] === picked) {
                    context.strokeStyle = ink;
                    context.lineWidth = 2;
                    context.beginPath();
                    context.arc(left, top, 11, 0, 2 * Math.PI);
                    context.stroke();
                }
            }
        }
    });
    return (
        <canvas ref={canvas} aria-hidden="true" className="pointer-events-none absolute inset-0 size-full" />
    );
}

/** Over the map while the layer is on: the kinds to show with their counts, and the way to the list. */
export function LayerBar({ layer, className }: { layer: EntityLayer; className?: string }) {
    const { t, format } = useI18n();
    const words = t.map.entities;
    const counts = layer.shown?.kinds ?? null;
    const unavailable = layer.live?.unavailable ?? null;
    const note: ReactNode =
        layer.failed !== null
            ? words.failed
            : unavailable === 'onesync'
              ? words.noOneSync
              : unavailable === 'game'
                ? words.otherGame
                : layer.live === null || layer.live.at === null
                  ? words.waiting
                  : layer.shown !== null && layer.shown.cell !== null
                    ? words.cells
                    : null;
    return (
        <div className={cn('flex min-w-0 flex-col items-end gap-2', className)}>
            <fieldset
                data-overlay
                className="m-0 flex max-w-full flex-wrap items-center gap-0.5 rounded-box border border-line bg-surface p-0.75 shadow-pop"
            >
                <legend className="sr-only">{words.shown}</legend>
                {KINDS.map((kind) => (
                    <button
                        key={kind}
                        type="button"
                        aria-pressed={layer.kinds.includes(kind)}
                        className="inline-flex h-6.5 items-center gap-1.75 rounded-[7px] px-2.25 text-small font-medium text-ink-3 transition-colors hover:text-ink aria-pressed:bg-raise aria-pressed:text-ink aria-pressed:shadow-[var(--elev-sm),0_0_0_1px_var(--line)] max-md:h-11"
                        onClick={() => layer.toggle(kind)}
                    >
                        <EntityMark kind={kind} />
                        {t.entities.filters[kind]}
                        {counts !== null && (
                            <span className="font-normal text-ink-3 max-md:hidden">
                                {format.number(counts[COUNT_KEYS[kind]])}
                            </span>
                        )}
                    </button>
                ))}
                <Link to="/entities" className={cn(buttonVariants({ size: 'sm', tone: 'ghost' }), 'h-6.5')}>
                    <List size={14} strokeWidth={1.5} aria-hidden="true" />
                    {words.list}
                </Link>
            </fieldset>
            {note !== null && (
                <output
                    data-overlay
                    className="max-w-88 rounded-box border border-line bg-surface px-3 py-1.5 text-meta text-ink-2 shadow-pop"
                >
                    {note}
                </output>
            )}
        </div>
    );
}

/** The picked entity: what the list says of it, and the same ways to act on it. */
export function EntityCard({
    layer,
    beside,
    canDelete,
    onDelete,
}: {
    layer: EntityLayer;
    /** Beside the map rather than over it. */
    beside: boolean;
    canDelete: boolean;
    onDelete: (item: EntityItem) => void;
}) {
    const { t } = useI18n();
    const words = t.map.entities;
    const item = layer.item;
    const never = item === null ? null : blocker(item);
    return (
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
                {item !== null && <EntityMark kind={item.kind} className="mt-1.75" />}
                <div className="flex min-w-0 flex-1 flex-col">
                    {item === null ? (
                        <span className="text-ui text-ink-2">
                            {layer.itemRead ? words.gone : t.common.loading}
                        </span>
                    ) : (
                        <>
                            <span className="text-card text-ink">{kindWord(t, item)}</span>
                            <span className="font-mono text-meta text-ink-3 [font-variant-ligatures:none]">
                                {modelHash(item.model)}
                            </span>
                        </>
                    )}
                </div>
                <Button tone="ghost" size="icon-sm" aria-label={t.map.close} onClick={() => layer.pick(null)}>
                    <X size={14} strokeWidth={1.5} aria-hidden="true" />
                </Button>
            </div>
            {item !== null && (
                <>
                    <div className="flex flex-wrap gap-1.5">
                        <Chip className="font-mono">{t.entities.netId(item.netId)}</Chip>
                        <Chip>{words.bucket(item.bucket)}</Chip>
                        {scripted(item) && <Chip>{words.script}</Chip>}
                        {item.occupied && <Chip tone="accent">{t.entities.playerInside}</Chip>}
                    </div>
                    <dl className="grid grid-cols-2 gap-x-3 gap-y-2">
                        <Fact term={t.entities.createdBy}>
                            {item.resource ??
                                (item.resourcePending ? t.entities.lookingUp : t.entities.noResource)}
                        </Fact>
                        <Fact term={t.entities.owner}>
                            {!item.owned
                                ? t.entities.noOwner
                                : layer.ownersShown && item.owner !== null
                                  ? t.entities.player(item.owner, item.ownerName)
                                  : t.entities.someOwner}
                        </Fact>
                        <Fact term={t.map.position} wide>
                            <CopyValue
                                value={`${item.x.toFixed(1)}, ${item.y.toFixed(1)}, ${item.z.toFixed(1)}`}
                                shown={`${item.x.toFixed(1)}, ${item.y.toFixed(1)}, ${item.z.toFixed(1)}`}
                                label={words.copyPosition}
                            />
                        </Fact>
                    </dl>
                    <div className="flex flex-wrap gap-2 [&>*]:flex-1">
                        <Link
                            to="/entities"
                            search={{ search: t.entities.netId(item.netId) }}
                            className={buttonVariants({ size: 'sm' })}
                        >
                            <List size={14} strokeWidth={1.5} aria-hidden="true" />
                            {words.inList}
                        </Link>
                        {canDelete && (
                            <Button
                                size="sm"
                                icon={Trash2}
                                className="text-danger-ink"
                                aria-label={t.entities.deleteOne(entityName(t, item))}
                                title={never === null ? undefined : t.entities.never[never]}
                                disabled={never !== null}
                                onClick={() => onDelete(item)}
                            >
                                {words.delete}
                            </Button>
                        )}
                    </div>
                </>
            )}
        </section>
    );
}

function Fact({ term, wide = false, children }: { term: string; wide?: boolean; children: ReactNode }) {
    return (
        <div className={cn('flex min-w-0 flex-col', wide && 'col-span-2')}>
            <dt className="text-meta text-ink-3">{term}</dt>
            <dd className="text-small text-ink [overflow-wrap:anywhere]">{children}</dd>
        </div>
    );
}
