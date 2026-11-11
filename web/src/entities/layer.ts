import type { EntityCell } from '../api/generated/EntityCell';
import type { EntityPoint } from '../api/generated/EntityPoint';
import type { MapBounds } from '../api/generated/MapBounds';

/** The sides a density cell may have, in game units, as the API takes them. */
export const CELLS = [25, 50, 100, 200, 500, 1000, 2000] as const;
/** About how wide a density cell is on screen. */
const CELL_PIXELS = 36;

/**
 * The side of a density cell for a zoom, in game units: about 36 px on screen, among the sides the
 * API takes, so that zooming does not ask for new cells at every step.
 */
export const cellFor = (unitsPerPixel: number) =>
    CELLS.find((cell) => cell >= unitsPerPixel * CELL_PIXELS) ?? 2000;

/**
 * The part of the map to ask the entities of, for a view: the squares of a grid the view touches,
 * the grid as coarse as the view is wide. The same answer serves while the map is dragged within a
 * square, and zooming in asks for less, down to where each entity can be drawn.
 */
export const boundsFor = (view: MapBounds): MapBounds => {
    const span = Math.max(view.east - view.west, view.north - view.south, 1);
    const square = 2 ** Math.floor(Math.log2(span));
    const west = Math.floor(view.west / square) * square;
    const south = Math.floor(view.south / square) * square;
    return {
        west,
        // Never a part without any width, which the API refuses.
        east: Math.max(Math.ceil(view.east / square) * square, west + square),
        south,
        north: Math.max(Math.ceil(view.north / square) * square, south + square),
    };
};

/** What a press on the map tells of itself: a place of the game and how far around it it reaches. */
type Reach = { x: number; y: number; reach: number };

/** The entity nearest to a press, within its reach. */
export const nearest = (points: readonly EntityPoint[], press: Reach): EntityPoint | null => {
    let found: EntityPoint | null = null;
    let least = press.reach * press.reach;
    for (const point of points) {
        const distance = (point[3] - press.x) ** 2 + (point[4] - press.y) ** 2;
        if (distance <= least) {
            least = distance;
            found = point;
        }
    }
    return found;
};

/** The cell a place of the game falls in, if the map counted any entity there. */
export const cellAt = (cells: readonly EntityCell[], side: number, x: number, y: number) => {
    const column = Math.floor(x / side);
    const row = Math.floor(y / side);
    return cells.find((cell) => cell[0] === column && cell[1] === row) ?? null;
};

/** The part of the game a cell covers. */
export const cellBounds = (cell: EntityCell, side: number): MapBounds => ({
    west: cell[0] * side,
    east: (cell[0] + 1) * side,
    south: cell[1] * side,
    north: (cell[1] + 1) * side,
});

/**
 * How strongly a cell is drawn, from 0 to 1: the fullest cell in view the strongest, on a
 * logarithmic scale so that a cell of a few entities still shows beside one of thousands.
 */
export const strength = (count: number, most: number) =>
    most <= 1 ? 1 : Math.min(1, Math.log(count + 1) / Math.log(most + 1));

/** A count as a cell has room for: `84`, `1.2k`, `12k`. */
export const shortCount = (count: number) => {
    if (count < 1000) {
        return String(count);
    }
    const thousands = count / 1000;
    return `${thousands < 10 ? thousands.toFixed(1).replace(/\.0$/, '') : Math.round(thousands)}k`;
};

/** A square around a place, to bring an entity into view with some of the map around it. */
export const around = (x: number, y: number, margin = 60): MapBounds => ({
    west: x - margin,
    east: x + margin,
    south: y - margin,
    north: y + margin,
});
