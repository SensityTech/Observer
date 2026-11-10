import type { MapBounds } from '../api/generated/MapBounds';
import type { MapImagery } from '../api/generated/MapImagery';

/** The styles of Rockstar's own map of GTA V, which its servers hold as tiles. */
export type TileStyle = Extract<MapImagery, 'atlas' | 'satellite' | 'roads'>;

export const TILE_STYLES: TileStyle[] = ['atlas', 'satellite', 'roads'];

export const isTileStyle = (imagery: MapImagery): imagery is TileStyle =>
    (TILE_STYLES as MapImagery[]).includes(imagery);

/** The name Rockstar's servers give each style. */
const SERVED_AS: Record<TileStyle, string> = { atlas: 'game', satellite: 'render', roads: 'print' };

/** The sea of each style, as its tiles paint it: it fills the map beyond them. */
export const SEA: Record<TileStyle, string> = { atlas: '#384950', satellite: '#0d2b4f', roads: '#4eb1d0' };

const TILE = 256;
/** The zoom the map's size and place are given at, and the deepest one the servers hold. */
const BASE_ZOOM = 4;
const MAX_ZOOM = 7;
/** At the base zoom the map is 8 tiles by 12. */
export const TILES_WORLD = { width: 8 * TILE, height: 12 * TILE };
/** Past this many tiles in view, a shallower zoom is drawn: no screen needs more. */
const MAX_TILES = 240;

/**
 * Where the map sits in the game: a position (x, y) falls on the pixel (KX·x + OX, KY·y + OY) of the
 * map at its base zoom, y growing to the south there and to the north in the game.
 */
const KX = 0.2231727578362635;
const OX = 941.1319005359636;
const KY = -0.2263214952251289;
const OY = 1903.7418159274296;

export const TILES_BOUNDS: MapBounds = {
    west: -OX / KX,
    east: (TILES_WORLD.width - OX) / KX,
    north: -OY / KY,
    south: (TILES_WORLD.height - OY) / KY,
};

export const tileUrl = (style: TileStyle, zoom: number, x: number, y: number) =>
    `https://s.rsg.sc/sc/images/games/GTAV/map/${SERVED_AS[style]}/${zoom}/${x}/${y}.jpg`;

/** A tile and the square it covers on the screen, edges rounded so that neighbours leave no seam. */
export type Tile = { key: string; url: string; left: number; top: number; width: number; height: number };

type View = { scale: number; x: number; y: number };
type Size = { width: number; height: number };

/** The zoom whose tiles are at least as fine as the screen, a little slack allowed, within what the servers hold. */
export const zoomFor = (scale: number, pixelRatio: number) =>
    Math.min(MAX_ZOOM, Math.max(0, BASE_ZOOM + Math.ceil(Math.log2(scale * pixelRatio) - 0.3)));

/** The tiles of `zoom` that show in the view, each placed on the screen. */
const tilesAt = (style: TileStyle, zoom: number, view: View, size: Size): Tile[] => {
    // What one tile of this zoom spans, in pixels of the base zoom.
    const span = TILE * 2 ** (BASE_ZOOM - zoom);
    const columns = Math.ceil(TILES_WORLD.width / span);
    const rows = Math.ceil(TILES_WORLD.height / span);
    const first = (edge: number) => Math.max(0, Math.floor(-edge / view.scale / span));
    const last = (edge: number, length: number, count: number) =>
        Math.min(count - 1, Math.floor((length - edge) / view.scale / span));
    const tiles: Tile[] = [];
    for (let y = first(view.y); y <= last(view.y, size.height, rows); y += 1) {
        for (let x = first(view.x); x <= last(view.x, size.width, columns); x += 1) {
            const left = Math.round(view.x + x * span * view.scale);
            const top = Math.round(view.y + y * span * view.scale);
            tiles.push({
                key: `${style}/${zoom}/${x}/${y}`,
                url: tileUrl(style, zoom, x, y),
                left,
                top,
                width: Math.round(view.x + (x + 1) * span * view.scale) - left,
                height: Math.round(view.y + (y + 1) * span * view.scale) - top,
            });
        }
    }
    return tiles;
};

/**
 * What the map draws of a style: the whole map as its one shallowest tile, which is there at once and
 * under everything, then the tiles of the zoom the view calls for.
 */
export const tilesIn = (
    style: TileStyle,
    view: View,
    size: Size,
    pixelRatio: number,
): { under: Tile; over: Tile[] } => {
    const whole = Math.round(TILE * 2 ** BASE_ZOOM * view.scale);
    const under = {
        key: `${style}/0/0/0`,
        url: tileUrl(style, 0, 0, 0),
        left: Math.round(view.x),
        top: Math.round(view.y),
        width: whole,
        height: whole,
    };
    let zoom = zoomFor(view.scale, pixelRatio);
    let over = zoom === 0 ? [] : tilesAt(style, zoom, view, size);
    while (over.length > MAX_TILES && zoom > 1) {
        zoom -= 1;
        over = tilesAt(style, zoom, view, size);
    }
    return { under, over };
};
