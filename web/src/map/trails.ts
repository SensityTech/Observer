/** A place a player was seen at, and when. */
export type TrailPoint = { x: number; y: number; at: number };

/** Where each player on the map went, by their id on the server, the oldest place first. */
export type Trails = ReadonlyMap<number, readonly TrailPoint[]>;

/** How far back a trail goes. */
export const TRAIL_MS = 5 * 60_000;
/** A player who moved less than this, in game units, has not moved: no place is added. */
const MIN_MOVE = 2;
/** Places kept of one player: one every other second, over the five minutes. */
const MAX_POINTS = 160;

/**
 * The trails with where the players are at `now`: a place for each one who moved, nothing older
 * than five minutes, and no trail for someone who left the map. Kept in the page alone: Observer
 * stores no position.
 */
export const extend = (
    trails: Trails,
    players: readonly { netId: number; x: number; y: number }[],
    now: number,
): Trails => {
    const next = new Map<number, readonly TrailPoint[]>();
    for (const player of players) {
        const kept = (trails.get(player.netId) ?? []).filter((point) => now - point.at <= TRAIL_MS);
        const last = kept.at(-1);
        const moved = last === undefined || Math.hypot(player.x - last.x, player.y - last.y) >= MIN_MOVE;
        next.set(
            player.netId,
            moved ? [...kept, { x: player.x, y: player.y, at: now }].slice(-MAX_POINTS) : kept,
        );
    }
    return next;
};
