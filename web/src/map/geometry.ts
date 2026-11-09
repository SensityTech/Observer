import type { MapBounds } from '../api/generated/MapBounds';
import type { MapSettings } from '../api/generated/MapSettings';

/** A place on the map as a fraction of its width from the west and of its height from the north. */
export type Fraction = { x: number; y: number };

/** Where a game position falls on the image: x grows to the east, y to the north in the game. */
export const toFraction = (bounds: MapBounds, x: number, y: number): Fraction => ({
    x: (x - bounds.west) / (bounds.east - bounds.west),
    y: (bounds.north - y) / (bounds.north - bounds.south),
});

/** A spot both on the image, in pixels, and in the game, where a player stood. */
export type Spot = { pixel: { x: number; y: number }; world: { x: number; y: number } };

/** The smallest distance two spots keep apart on each axis, in pixels and in game units. */
const MIN_SPREAD = 50;

/**
 * The image's edges in the game from two spots and its size: each axis apart, as the image is drawn
 * north up with no rotation. `null` when the spots are too close on either axis to tell the scale.
 */
export const boundsFrom = (first: Spot, second: Spot, width: number, height: number): MapBounds | null => {
    const pixelX = second.pixel.x - first.pixel.x;
    const pixelY = second.pixel.y - first.pixel.y;
    const worldX = second.world.x - first.world.x;
    const worldY = second.world.y - first.world.y;
    if (
        Math.abs(pixelX) < MIN_SPREAD ||
        Math.abs(pixelY) < MIN_SPREAD ||
        Math.abs(worldX) < MIN_SPREAD ||
        Math.abs(worldY) < MIN_SPREAD
    ) {
        return null;
    }
    const perPixelX = worldX / pixelX;
    // The image's y grows to the south, the game's to the north.
    const perPixelY = -worldY / pixelY;
    if (perPixelX <= 0 || perPixelY <= 0) {
        return null;
    }
    const west = first.world.x - first.pixel.x * perPixelX;
    const north = first.world.y + first.pixel.y * perPixelY;
    return {
        west: round(west),
        east: round(west + width * perPixelX),
        south: round(north - height * perPixelY),
        north: round(north),
    };
};

const round = (value: number) => Math.round(value * 10) / 10;

export type Tier = 'new' | 'regular' | 'veteran' | 'unconfirmed';

/** A player's colour on the map: unconfirmed until their identity is, then by play time. */
export const tierOf = (
    totalMs: number | null,
    settings: Pick<MapSettings, 'newHours' | 'veteranHours'>,
): Tier => {
    if (totalMs === null) {
        return 'unconfirmed';
    }
    const hours = totalMs / 3_600_000;
    if (hours < settings.newHours) {
        return 'new';
    }
    return hours >= settings.veteranHours ? 'veteran' : 'regular';
};

/** The span the grid of game coordinates covers when no image is placed: GTA V's world and around it. */
export const GRID_BOUNDS: MapBounds = { west: -4500, east: 4500, south: -4500, north: 8500 };

/** A round step of game units for the grid and the scale bar, about `target` long. */
export const niceStep = (target: number) => {
    const power = 10 ** Math.floor(Math.log10(Math.max(target, 1)));
    for (const step of [1, 2, 5, 10]) {
        if (step * power >= target) {
            return step * power;
        }
    }
    return 10 * power;
};
