import type { RunMarker } from '../api/generated/RunMarker';

export type Side = 'left' | 'right' | 'hidden';

/** A marker placed on the plot: its instant at `x`, how wide its label is, both in pixels, and how
 * much it matters next to the others. */
export type Placed = { x: number; width: number; weight: number };

/**
 * Which side of its instant each marker's label goes: left of it by default, right of it when the
 * left would cross the plot's edge or another label. The weightier labels are placed first, then
 * the others in time order; a label that fits on neither side is hidden, and its line stays.
 */
export const sides = (placed: readonly Placed[], plotWidth: number): Side[] => {
    const order = placed
        .map((marker, index) => ({ ...marker, index }))
        .toSorted((a, b) => b.weight - a.weight || a.x - b.x);
    const taken: [number, number][] = [];
    const free = ([start, end]: [number, number]) =>
        start >= 0 && end <= plotWidth && taken.every(([from, to]) => end <= from || start >= to);
    const result: Side[] = placed.map(() => 'hidden');
    for (const { x, width, index } of order) {
        const left: [number, number] = [x - width, x];
        const right: [number, number] = [x, x + width];
        if (free(left)) {
            taken.push(left);
            result[index] = 'left';
        } else if (free(right)) {
            taken.push(right);
            result[index] = 'right';
        }
    }
    return result;
};

/** Where a marker sits on the chart: the start of its bucket, the only times the grid has. */
export const bucketOf = (marker: RunMarker, from: number, step: number) =>
    from + Math.floor((marker.at - from) / step) * step;
