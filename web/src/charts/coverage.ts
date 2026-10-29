import type { MetricsSeries } from '../api/generated/MetricsSeries';

/** A stretch of buckets, from the first one's start to the last one's end, in Unix seconds. */
export type Period = { from: number; to: number };

/** Observer sampled the bucket and the server was never online or degraded in it: a start counts. */
const isDown = (page: MetricsSeries, index: number) =>
    (page.observed[index] ?? 0) > 0 && (page.online[index] ?? 0) === 0;

/**
 * The stretches the server was down over the page. A bucket Observer did not sample at all is a
 * gap, not a stretch down.
 */
export const downPeriods = (page: MetricsSeries): Period[] => {
    const periods: Period[] = [];
    let open: Period | null = null;
    for (let index = 0; index < page.buckets; index += 1) {
        const from = page.from + index * page.step;
        if (isDown(page, index)) {
            if (open === null) {
                open = { from, to: from + page.step };
                periods.push(open);
            } else {
                open.to = from + page.step;
            }
        } else {
            open = null;
        }
    }
    return periods;
};

/** How long the server was down over the page, in seconds. */
export const downTime = (periods: readonly Period[]) =>
    periods.reduce((sum, period) => sum + period.to - period.from, 0);
