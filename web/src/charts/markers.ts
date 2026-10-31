import type { RunMarker } from '../api/generated/RunMarker';
import { formatTimestamp } from '../i18n/format';

/** A change of FXServer's build at a start, which the drops chart marks beside the runs. */
export type BuildMarker = { kind: 'build'; at: number; build: number | null };

/** What a chart marks above its plot: a moment of the server's runs, or a build change. */
export type ChartMarker = RunMarker | BuildMarker;

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
export const bucketOf = (marker: ChartMarker, from: number, step: number) =>
    from + Math.floor((marker.at - from) / step) * step;

/** A marker on a narrow plot: where its line falls, and whether it is a crash. */
export type Dot = { x: number; crash: boolean };

/** Markers sharing one icon: the members, by index, and the one whose icon stands for them all. */
export type Cluster = { members: number[]; shown: number };

/**
 * Markers whose icons would stand less than `gap` pixels apart, chained in order of `x`, share one
 * icon: a crash's when one of them is a crash, the newest's otherwise.
 */
export const clusters = (dots: readonly Dot[], gap: number): Cluster[] => {
    const order = dots.map((dot, index) => ({ ...dot, index })).toSorted((a, b) => a.x - b.x);
    const groups: (typeof order)[] = [];
    for (const dot of order) {
        const group = groups.at(-1);
        const last = group?.at(-1);
        if (group !== undefined && last !== undefined && dot.x - last.x < gap) {
            group.push(dot);
        } else {
            groups.push([dot]);
        }
    }
    return groups.map((group) => {
        const crash = group.findLast((dot) => dot.crash);
        const shown = crash ?? group.at(-1);
        return { members: group.map((dot) => dot.index), shown: shown?.index ?? 0 };
    });
};

const pad = (value: number) => String(value).padStart(2, '0');

/** A marker's time on the chart: hours and minutes, local. */
export const clock = (seconds: number) => {
    const date = new Date(seconds * 1000);
    return `${pad(date.getHours())}:${pad(date.getMinutes())}`;
};

/**
 * A time a chart words: hours and minutes, and the day too over a week or a month (buckets of 15
 * minutes or more), where two events days apart can share their hours and minutes.
 */
export const chartTime = (seconds: number, step: number) =>
    step >= 900 ? formatTimestamp(new Date(seconds * 1000), { seconds: false }) : clock(seconds);
