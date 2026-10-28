import { cn } from '../lib/cn';

const WIDTH = 100;
const HEIGHT = 28;
/** Room for the stroke at the top and the bottom of the box. */
const PAD = 2;

export type SparklinePath = { line: string; area: string; flat: boolean };

/**
 * The line and the area under it in a 100 × 28 box, from the lowest value to the highest (or to
 * `max`, shared by sparklines that compare). A gap breaks the path into runs; a run of one point is
 * a dot. Null when nothing was measured.
 */
export const sparklinePath = (values: readonly (number | null)[], max?: number): SparklinePath | null => {
    const measured = values.filter((value): value is number => value !== null);
    if (measured.length === 0) {
        return null;
    }
    const low = Math.min(...measured);
    const high = Math.max(max ?? -Infinity, ...measured);
    const flat = high === low;
    const step = values.length > 1 ? WIDTH / (values.length - 1) : 0;
    const x = (index: number) => round(index * step);
    const y = (value: number) =>
        flat ? HEIGHT / 2 : round(HEIGHT - PAD - ((value - low) / (high - low)) * (HEIGHT - 2 * PAD));

    const runs: [number, number][][] = [];
    let run: [number, number][] = [];
    values.forEach((value, index) => {
        if (value === null) {
            if (run.length > 0) {
                runs.push(run);
            }
            run = [];
        } else {
            run.push([x(index), y(value)]);
        }
    });
    if (run.length > 0) {
        runs.push(run);
    }
    const line = runs
        .map((points) => {
            const path = points.map(([px, py], at) => `${at === 0 ? 'M' : 'L'}${px} ${py}`).join('');
            return points.length === 1 ? `${path}h0.01` : path;
        })
        .join('');
    const area = runs
        .filter((points) => points.length > 1)
        .map((points) => {
            const first = points[0]?.[0] ?? 0;
            const last = points.at(-1)?.[0] ?? first;
            return `M${first} ${HEIGHT}${points.map(([px, py]) => `L${px} ${py}`).join('')}L${last} ${HEIGHT}Z`;
        })
        .join('');
    return { line, area, flat };
};

const round = (value: number) => Math.round(value * 100) / 100;

/** A tile's trend: 32 px high, the tile's width. A flat trend is grey and says nothing to a reader. */
export function Sparkline({
    values,
    label,
    max,
    className,
}: {
    values: readonly (number | null)[];
    label: string;
    max?: number;
    className?: string;
}) {
    const path = sparklinePath(values, max);
    const flat = path === null || path.flat;
    return (
        <svg
            className={cn(
                'block h-8 w-full overflow-visible',
                flat ? 'text-chart-muted' : 'text-accent',
                className,
            )}
            viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
            preserveAspectRatio="none"
            {...(flat ? { 'aria-hidden': true } : { role: 'img', 'aria-label': label })}
        >
            {path !== null && (
                <>
                    <path d={path.area} fill="currentColor" fillOpacity={0.1} stroke="none" />
                    <path
                        d={path.line}
                        fill="none"
                        stroke="currentColor"
                        strokeWidth={1.5}
                        strokeLinejoin="round"
                        strokeLinecap="round"
                        vectorEffect="non-scaling-stroke"
                    />
                </>
            )}
        </svg>
    );
}
