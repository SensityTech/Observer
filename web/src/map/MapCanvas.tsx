import { Maximize, Minus, Plus } from 'lucide-react';
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from 'react';

import type { MapBounds } from '../api/generated/MapBounds';
import { Button } from '../components/Button';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { GRID_BOUNDS, niceStep, toFraction, type Tier } from './geometry';

/** A player as the map draws them: where, in which colour, and what a reader hears. */
export type Dot = { id: number; x: number; y: number; tier: Tier; name: string; label: string };

/** The marks of each colour, which also differ in shape: a colour never says it alone. */
export const TIER_MARK: Record<Tier, string> = {
    new: 'size-3 rounded-full border-[2.5px] border-warning bg-surface',
    regular: 'size-3 rounded-full bg-accent ring-2 ring-surface',
    veteran: 'size-3 rounded-full bg-success shadow-[0_0_0_2px_var(--surface),0_0_0_3.5px_var(--success)]',
    unconfirmed: 'size-2 rounded-full bg-ink-3 ring-2 ring-surface',
};

const MIN_ZOOM = 0.05;
const MAX_ZOOM = 16;
const STEP = 1.5;
/** Room kept around the players when the map shows them all. */
const MARGIN = 48;
/** The size of the world without an image, kept to the grid's proportions. */
const GRID_WIDTH = 1000;

type View = { scale: number; x: number; y: number };

/**
 * The map: the owner's image placed in the game, or a grid of game coordinates, dragged and zoomed,
 * with a mark for each player that keeps its size whatever the zoom.
 */
export function MapCanvas({
    image,
    bounds,
    dots,
    selected,
    names,
    focus,
    onSelect,
    children,
    className,
}: {
    /** A `blob:` address and the image's size in pixels, or null for the grid. */
    image: { url: string; width: number; height: number } | null;
    /** Where the image's edges lie in the game; null draws the grid. */
    bounds: MapBounds | null;
    dots: Dot[];
    selected: number | null;
    names: boolean;
    /** A player to bring into view, and a count that changes each time it is asked. */
    focus: { id: number; at: number } | null;
    onSelect: (id: number | null) => void;
    /** Drawn over the map: notices and the selected player's card. */
    children?: ReactNode;
    className?: string;
}) {
    const { t } = useI18n();
    const words = t.map;
    const box = useRef<HTMLDivElement>(null);
    const [size, setSize] = useState({ width: 0, height: 0 });
    const placed = image !== null && bounds !== null;
    const area = placed ? bounds : GRID_BOUNDS;
    const world = placed
        ? { width: image.width, height: image.height }
        : {
              width: GRID_WIDTH,
              height:
                  (GRID_WIDTH * (GRID_BOUNDS.north - GRID_BOUNDS.south)) /
                  (GRID_BOUNDS.east - GRID_BOUNDS.west),
          };
    const [view, setView] = useState<View | null>(null);
    const drag = useRef<{ pointer: number; x: number; y: number; moved: boolean } | null>(null);

    useLayoutEffect(() => {
        const element = box.current;
        if (element === null) {
            return;
        }
        const observer = new ResizeObserver(([entry]) => {
            if (entry !== undefined) {
                setSize({ width: entry.contentRect.width, height: entry.contentRect.height });
            }
        });
        observer.observe(element);
        return () => observer.disconnect();
    }, []);

    /** The view that shows a rectangle of the world, in world pixels, whole. */
    const framing = useCallback(
        (left: number, top: number, right: number, bottom: number): View => {
            const spanX = Math.max(right - left, 1);
            const spanY = Math.max(bottom - top, 1);
            const scale = clamp(
                Math.min((size.width - 2 * MARGIN) / spanX, (size.height - 2 * MARGIN) / spanY),
                MIN_ZOOM,
                MAX_ZOOM,
            );
            return {
                scale,
                x: size.width / 2 - ((left + right) / 2) * scale,
                y: size.height / 2 - ((top + bottom) / 2) * scale,
            };
        },
        [size],
    );

    const worldOf = useCallback(
        (dot: Dot) => {
            const fraction = toFraction(area, dot.x, dot.y);
            return { x: fraction.x * world.width, y: fraction.y * world.height };
        },
        [area, world.width, world.height],
    );

    const fit = useCallback(() => {
        if (size.width === 0) {
            return;
        }
        if (dots.length === 0) {
            setView(framing(0, 0, world.width, world.height));
            return;
        }
        const points = dots.map(worldOf);
        const xs = points.map((point) => point.x);
        const ys = points.map((point) => point.y);
        // A single player is shown with some of the map around them.
        const pad = dots.length === 1 ? world.width / 12 : 0;
        setView(
            framing(
                Math.min(...xs) - pad,
                Math.min(...ys) - pad,
                Math.max(...xs) + pad,
                Math.max(...ys) + pad,
            ),
        );
    }, [dots, framing, size.width, world.width, world.height, worldOf]);

    // The first view shows every player, or the whole map; a new image starts over.
    const framedFor = useRef<string | null>(null);
    useEffect(() => {
        const key = `${image?.url ?? 'grid'}:${placed}`;
        if (size.width > 0 && framedFor.current !== key) {
            framedFor.current = key;
            fit();
        }
    }, [fit, image?.url, placed, size.width]);

    // Only a new ask moves the map, not each new position: what it reads stays out of the effect's
    // dependencies.
    const latest = useRef({ dots, worldOf, size });
    useLayoutEffect(() => {
        latest.current = { dots, worldOf, size };
    });
    useEffect(() => {
        const { dots: now, worldOf: place, size: frame } = latest.current;
        const dot = focus === null ? undefined : now.find((each) => each.id === focus.id);
        if (dot === undefined || frame.width === 0) {
            return;
        }
        const point = place(dot);
        setView((current) => {
            const scale = Math.max(current?.scale ?? 1, 1);
            return { scale, x: frame.width / 2 - point.x * scale, y: frame.height / 2 - point.y * scale };
        });
    }, [focus]);

    const zoom = (factor: number, at = { x: size.width / 2, y: size.height / 2 }) =>
        setView((current) => {
            if (current === null) {
                return current;
            }
            const scale = clamp(current.scale * factor, MIN_ZOOM, MAX_ZOOM);
            const ratio = scale / current.scale;
            return { scale, x: at.x - (at.x - current.x) * ratio, y: at.y - (at.y - current.y) * ratio };
        });

    // A wheel over the map zooms it rather than scrolling the page.
    useEffect(() => {
        const element = box.current;
        if (element === null) {
            return;
        }
        const wheel = (event: WheelEvent) => {
            event.preventDefault();
            const rect = element.getBoundingClientRect();
            zoom(event.deltaY < 0 ? 1.2 : 1 / 1.2, {
                x: event.clientX - rect.left,
                y: event.clientY - rect.top,
            });
        };
        element.addEventListener('wheel', wheel, { passive: false });
        return () => element.removeEventListener('wheel', wheel);
    });

    // Dragging moves the map; a press that does not move clears the selection. On the element itself,
    // so that a drag that leaves it carries on.
    const select = useRef(onSelect);
    useLayoutEffect(() => {
        select.current = onSelect;
    });
    useEffect(() => {
        const element = box.current;
        if (element === null) {
            return;
        }
        const down = (event: PointerEvent) => {
            if (
                event.button !== 0 ||
                (event.target as HTMLElement).closest('button, a, [data-overlay]') !== null
            ) {
                return;
            }
            element.setPointerCapture(event.pointerId);
            drag.current = { pointer: event.pointerId, x: event.clientX, y: event.clientY, moved: false };
        };
        const move = (event: PointerEvent) => {
            const current = drag.current;
            if (current === null || current.pointer !== event.pointerId) {
                return;
            }
            const dx = event.clientX - current.x;
            const dy = event.clientY - current.y;
            drag.current = {
                ...current,
                x: event.clientX,
                y: event.clientY,
                moved: current.moved || Math.abs(dx) + Math.abs(dy) > 2,
            };
            setView((shown) => (shown === null ? shown : { ...shown, x: shown.x + dx, y: shown.y + dy }));
        };
        const up = (event: PointerEvent) => {
            const current = drag.current;
            drag.current = null;
            if (current !== null && !current.moved) {
                select.current(null);
            }
            if (element.hasPointerCapture(event.pointerId)) {
                element.releasePointerCapture(event.pointerId);
            }
        };
        element.addEventListener('pointerdown', down);
        element.addEventListener('pointermove', move);
        element.addEventListener('pointerup', up);
        element.addEventListener('pointercancel', up);
        return () => {
            element.removeEventListener('pointerdown', down);
            element.removeEventListener('pointermove', move);
            element.removeEventListener('pointerup', up);
            element.removeEventListener('pointercancel', up);
        };
    }, []);

    const shown = view ?? { scale: 0, x: 0, y: 0 };
    const screen = (dot: Dot) => {
        const point = worldOf(dot);
        return { left: shown.x + point.x * shown.scale, top: shown.y + point.y * shown.scale };
    };

    return (
        <div
            ref={box}
            className={cn(
                'relative cursor-grab touch-none overflow-hidden bg-surface-2 select-none active:cursor-grabbing',
                className,
            )}
        >
            {view !== null && (
                <div
                    aria-hidden="true"
                    className="pointer-events-none absolute top-0 left-0 origin-top-left"
                    style={{
                        width: world.width,
                        height: world.height,
                        transform: `translate(${view.x}px, ${view.y}px) scale(${view.scale})`,
                    }}
                >
                    {placed ? (
                        <img
                            src={image.url}
                            alt=""
                            draggable={false}
                            className="block size-full max-w-none"
                        />
                    ) : (
                        <div className="size-full bg-bg-subtle" />
                    )}
                </div>
            )}
            {view !== null && !placed && <Grid view={view} world={world} area={area} size={size} />}
            {view !== null &&
                dots.map((dot) => {
                    const { left, top } = screen(dot);
                    const isSelected = dot.id === selected;
                    return (
                        <div key={dot.id} className="absolute" style={{ left, top }}>
                            <button
                                type="button"
                                aria-label={dot.label}
                                aria-pressed={isSelected}
                                className="absolute grid size-6 -translate-1/2 cursor-pointer place-items-center rounded-full focus-visible:outline-2 focus-visible:outline-accent max-md:size-10"
                                onClick={() => onSelect(isSelected ? null : dot.id)}
                            >
                                <span
                                    className={cn(
                                        TIER_MARK[dot.tier],
                                        isSelected && 'outline-2 outline-offset-4 outline-ink',
                                    )}
                                />
                            </button>
                            {(names || isSelected) && (
                                <span
                                    aria-hidden="true"
                                    className="pointer-events-none absolute -translate-x-1/2 -translate-y-[calc(100%+12px)] rounded-xs border border-line-strong bg-surface px-1.5 py-0.5 text-caption font-medium whitespace-nowrap text-ink shadow-sm"
                                >
                                    {dot.name}
                                </span>
                            )}
                        </div>
                    );
                })}
            {view !== null && <Scale scale={view.scale} area={area} world={world} />}
            <fieldset
                data-overlay
                className="absolute right-3 bottom-3 m-0 flex flex-col overflow-hidden rounded-control border border-line-strong bg-surface p-0 shadow-pop"
            >
                <legend className="sr-only">{words.zoom}</legend>
                <Button
                    tone="ghost"
                    size="icon"
                    aria-label={words.zoomIn}
                    className="rounded-none"
                    onClick={() => zoom(STEP)}
                >
                    <Plus size={16} strokeWidth={1.5} aria-hidden="true" />
                </Button>
                <Button
                    tone="ghost"
                    size="icon"
                    aria-label={words.zoomOut}
                    className="rounded-none border-t border-line"
                    onClick={() => zoom(1 / STEP)}
                >
                    <Minus size={16} strokeWidth={1.5} aria-hidden="true" />
                </Button>
                <Button
                    tone="ghost"
                    size="icon"
                    aria-label={words.fit}
                    className="rounded-none border-t border-line"
                    onClick={fit}
                >
                    <Maximize size={16} strokeWidth={1.5} aria-hidden="true" />
                </Button>
            </fieldset>
            {children}
        </div>
    );
}

/** The lines of game coordinates in view, with their values along the top and left edges. */
function Grid({
    view,
    world,
    area,
    size,
}: {
    view: View;
    world: { width: number; height: number };
    area: MapBounds;
    size: { width: number; height: number };
}) {
    const unitsPerPixel = (area.east - area.west) / (world.width * view.scale);
    const step = niceStep(unitsPerPixel * 96);
    const gameX = (screenX: number) =>
        area.west + ((screenX - view.x) / view.scale / world.width) * (area.east - area.west);
    const gameY = (screenY: number) =>
        area.north - ((screenY - view.y) / view.scale / world.height) * (area.north - area.south);
    const screenX = (x: number) =>
        view.x + ((x - area.west) / (area.east - area.west)) * world.width * view.scale;
    const screenY = (y: number) =>
        view.y + ((area.north - y) / (area.north - area.south)) * world.height * view.scale;
    const xs: number[] = [];
    for (let x = Math.ceil(gameX(0) / step) * step; x <= gameX(size.width); x += step) {
        xs.push(x);
    }
    const ys: number[] = [];
    for (let y = Math.floor(gameY(0) / step) * step; y >= gameY(size.height); y -= step) {
        ys.push(y);
    }
    return (
        <div aria-hidden="true" className="pointer-events-none absolute inset-0">
            {xs.map((x) => (
                <div
                    key={`x${x}`}
                    className="absolute top-0 bottom-0 w-px bg-line"
                    style={{ left: screenX(x) }}
                >
                    <span className="absolute top-1.5 left-1 font-mono text-micro text-ink-3">{x}</span>
                </div>
            ))}
            {ys.map((y) => (
                <div
                    key={`y${y}`}
                    className="absolute right-0 left-0 h-px bg-line"
                    style={{ top: screenY(y) }}
                >
                    <span className="absolute top-1 left-1.5 font-mono text-micro text-ink-3">{y}</span>
                </div>
            ))}
        </div>
    );
}

/** A bar of a round length in game units, which are metres. */
function Scale({ scale, area, world }: { scale: number; area: MapBounds; world: { width: number } }) {
    const { t } = useI18n();
    const pixelsPerUnit = (world.width * scale) / (area.east - area.west);
    const metres = niceStep(80 / pixelsPerUnit);
    return (
        <div
            aria-hidden="true"
            className="pointer-events-none absolute bottom-3.5 left-3 flex flex-col gap-1"
        >
            <span
                className="h-1.5 border-x-[1.5px] border-b-[1.5px] border-ink-3"
                style={{ width: metres * pixelsPerUnit }}
            />
            <span className="font-mono text-meta text-ink-3">{t.map.scale(metres.toLocaleString('en'))}</span>
        </div>
    );
}

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));
