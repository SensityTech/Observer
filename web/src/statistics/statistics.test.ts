import { describe, expect, it } from 'vitest';

import type { DropSeries } from '../api/generated/DropSeries';
import type { FrameDistribution } from '../api/generated/FrameDistribution';
import type { Growth } from '../api/generated/Growth';
import type { PlayerHours } from '../api/generated/PlayerHours';
import type { ScheduleSettings } from '../api/generated/ScheduleSettings';
import type { Timeline } from '../api/generated/Timeline';
import type { TimelineEvent } from '../api/generated/TimelineEvent';
import { en } from '../i18n/en';
import { restartAdvice } from './advice';
import { eventsAround, eventWords, linksIn, startOf } from './events';
import { intervalLabels, p99Change } from './insights/FramesCard';
import { survivalPath } from './insights/SessionsCard';
import { calendar } from './insights/StabilityCard';
import { bucketAt, cpuStack, forecast, frameLine, seriesOf } from './lanes';
import { parseStatisticsSearch, timelineWindow } from './search';
import { labelSpots } from './TimelineChart';

const timeline = (
    series: Partial<Record<string, (number | null)[]>>,
    frames: Timeline['frames'] = null,
): Timeline => {
    const buckets = Object.values(series)[0]?.length ?? 0;
    return {
        from: 1000,
        step: 60,
        buckets,
        last: null,
        series: Object.entries(series).map(([name, values]) => ({
            series: name as Timeline['series'][number]['series'],
            values: values ?? [],
        })),
        observed: Array.from({ length: buckets }, () => 12),
        online: Array.from({ length: buckets }, () => 12),
        frames,
        usual: null,
    };
};

describe('the lanes', () => {
    it('should read each series once, in the lanes order', () => {
        expect(seriesOf(['players', 'frames', 'players'])).toEqual([
            'players',
            'mainTick',
            'hitchesMain',
            'hitchMax',
        ]);
    });

    it('should stack the CPU groups, and leave a bucket without them a gap', () => {
        const stack = cpuStack(
            timeline({
                mainThreadCpu: [50, null],
                networkThreadCpu: [10, null],
                syncThreadCpu: [5, null],
                jsThreadsCpu: [3, null],
                monoThreadsCpu: [0, null],
                otherThreadsCpu: [2, null],
            }),
        );

        expect(stack.main.map((row) => row.value)).toEqual([50, null]);
        expect(stack.server.map((row) => row.value)).toEqual([65, null]);
        expect(stack.all.map((row) => row.value)).toEqual([70, null]);
    });

    it('should draw the median where percentiles were kept, the mean before them', () => {
        const kept = timeline(
            { mainTick: [3, 4] },
            { p50: [1.5, 2], p95: [5, 6], p99: [9, 12], ticks: [100, 100], floor: 1, top: 250, since: 1000 },
        );
        const before = timeline(
            { mainTick: [3, 4] },
            {
                p50: [null, null],
                p95: [null, null],
                p99: [null, null],
                ticks: [0, 0],
                floor: null,
                top: null,
                since: null,
            },
        );

        expect(frameLine(kept)).toEqual({
            kind: 'median',
            rows: [
                { time: 1000, value: 1.5 },
                { time: 1060, value: 2 },
            ],
        });
        expect(frameLine(before).kind).toBe('mean');
        expect(frameLine(before).rows.map((row) => row.value)).toEqual([3, 4]);
    });

    it('should place a time in its bucket, never off the grid', () => {
        const grid = timeline({ players: [1, 2, 3] });

        expect(bucketAt(grid, 1000)).toBe(0);
        expect(bucketAt(grid, 1119)).toBe(1);
        expect(bucketAt(grid, 999_999)).toBe(2);
        expect(bucketAt(grid, 0)).toBe(0);
    });

    it('should widen the forecast band away from the fitted minutes', () => {
        const growth: Growth = {
            perHour: 3600,
            fit: 0.9,
            start: 100,
            points: 60,
            leak: false,
            residual: 2,
            slopeError: 360,
            center: 1800,
        };

        const { trend, cone } = forecast(growth, 0, 3600, 7200, 1800);

        expect(trend[0]).toEqual({ time: 0, value: 100 });
        expect(trend.at(-1)).toEqual({ time: 7200, value: 7300 });
        const widths = cone.map((point) => (point.max ?? 0) - (point.min ?? 0));
        expect(widths[0]).toBeLessThan(widths.at(-1) ?? 0);
        expect(cone[0]?.time).toBe(3600);
    });
});

const at = (key: string, seconds: number, detail: TimelineEvent['detail'], until: number | null = null) => ({
    key,
    at: seconds,
    until,
    detail,
});

const link = (wave: string) => ({
    key: `start:4>${wave}`,
    change: 'start:4',
    wave,
    after: 60,
    names: null,
    dismissed: false,
});

const schedule = (times: string[], mode: ScheduleSettings['mode'] = 'fixedTimes'): ScheduleSettings => ({
    enabled: true,
    mode,
    times,
    intervalMinutes: 360,
    timeZone: null,
    warnings: [],
    skipAfterStart: 0,
});

const period = (p99: number | null) => ({
    from: 0,
    to: 1,
    ticks: 100,
    shares: null,
    p50: 1,
    p95: 5,
    p99,
    overBudget: null,
});

const distribution = (now: number | null, before: number | null): FrameDistribution => ({
    range: '7d',
    bounds: [1, 2],
    budget: 50,
    now: period(now),
    before: period(before),
    since: null,
});

describe('the events', () => {
    const words = {
        duration: (seconds: number) => `${seconds}s`,
        clock: () => '13:10',
        ms: (ms: number) => `${ms} ms`,
    };

    it('should find what happened within a quarter hour of the cursor, or a 48th of a long window', () => {
        const events = [
            at('a', 1000, { kind: 'stop' }),
            at('b', 5000, { kind: 'stop' }),
            at('wave', 10_000, { kind: 'incident', drops: 5, hitchMs: null, players: 30 }, 12_000),
        ];

        expect(eventsAround(events, 1500, 3600).map((event) => event.key)).toEqual(['a']);
        expect(eventsAround(events, 12_500, 3600).map((event) => event.key)).toEqual(['wave']);
        expect(eventsAround(events, 3000, 30 * 86_400).map((event) => event.key)).toEqual(['a', 'b', 'wave']);
    });

    it('should say a wave and a group of writes in words', () => {
        const wave = at(
            'wave:4:12',
            1000,
            {
                kind: 'crashWave',
                signatureId: 4,
                signature: null,
                label: null,
                crashes: 14,
                players: 9,
                resource: null,
            },
            2620,
        );
        const files = at('files:9', 900, {
            kind: 'files',
            files: 3,
            resources: ['sensity_garage'],
            by: ['alice'],
        });

        expect(eventWords(en, wave, words)).toBe('14 crashes from 9 players in 1620s, one signature');
        expect(eventWords(en, files, words)).toBe('sensity_garage: 3 files changed by alice');
    });

    it('should keep the links of the window, newest wave first, and read a start from its key', () => {
        const events = [
            at('start:4', 100, { kind: 'stop' }),
            at('wave:1:1', 200, { kind: 'stop' }),
            at('wave:2:2', 900, { kind: 'stop' }),
        ];

        expect(
            linksIn([link('wave:1:1'), link('wave:2:2'), link('wave:3:3')], events).map((each) => each.wave),
        ).toEqual(['wave:2:2', 'wave:1:1', 'wave:3:3']);
        expect(startOf('start:12')).toBe(12);
        expect(startOf('files:12')).toBeNull();
    });

    it('should leave a label out where the next event is too close', () => {
        expect(labelSpots([0, 40, 400, 700])).toEqual([false, true, true, true]);
        expect(labelSpots([0, 200, 260])).toEqual([true, false, true]);
    });
});

describe('the best time to restart', () => {
    const daily = Array.from({ length: 24 }, (_, hour) => (hour === 6 ? 4 : hour === 18 ? 96 : 50));
    const hours: PlayerHours = {
        zone: 'Europe/Paris',
        from: 0,
        to: 1,
        players: Array.from({ length: 168 }, (_, cell) => daily[cell % 24] ?? null),
        measured: Array.from({ length: 168 }, () => 4),
        daily,
        quietest: 6,
    };

    it('should move the busiest planned time to the quietest hour when that is clearly quieter', () => {
        expect(restartAdvice(hours, schedule(['18:00', '12:00']))).toEqual({
            kind: 'move',
            time: '18:00',
            players: 96,
            quiet: 6,
            after: 4,
        });
    });

    it('should say when the quietest hour is planned already, close, on an interval, or not planned', () => {
        expect(restartAdvice(hours, schedule(['06:30'])).kind).toBe('already');
        expect(
            restartAdvice(
                { ...hours, daily: daily.map((value, hour) => (hour === 12 ? 5 : value)) },
                schedule(['12:00']),
            ).kind,
        ).toBe('close');
        expect(restartAdvice(hours, schedule([], 'interval')).kind).toBe('interval');
        expect(restartAdvice(hours, null).kind).toBe('quiet');
        expect(restartAdvice({ ...hours, quietest: null }, schedule(['18:00'])).kind).toBe('none');
    });
});

describe('the insights', () => {
    it('should name the frame intervals from under the first bound to beyond the last', () => {
        const words = {
            under: (ms: string) => `< ${ms}`,
            beyond: (ms: string) => `> ${ms}`,
            interval: (from: string, to: string) => `${from}–${to}`,
        };

        expect(intervalLabels([1, 2, 250], String, words)).toEqual(['< 1', '1–2', '2–250', '> 250']);
    });

    it('should tell a better or worse p99 only past 15 percent', () => {
        expect(p99Change(distribution(7.8, 10))?.tone).toBe('better');
        expect(p99Change(distribution(12, 10))?.tone).toBe('worse');
        expect(p99Change(distribution(10.5, 10))?.tone).toBe('same');
        expect(p99Change(distribution(10, null))).toBeNull();
    });

    it('should draw survival as steps', () => {
        expect(survivalPath([1, 0.5])).toBe('M40 20 H44.583333333333336 V105.0');
    });

    it('should lay the days out a column a week from Monday', () => {
        // Sunday 2 Nov 2025, then Monday 3 and Tuesday 4, at midnight UTC.
        const series = {
            starts: [1_762_041_600, 1_762_128_000, 1_762_214_400],
            rates: [2, null, 3.5],
        } as DropSeries;

        expect(calendar(series, 'UTC').map(({ column, row, rate }) => [column, row, rate])).toEqual([
            [0, 6, 2],
            [1, 0, null],
            [1, 1, 3.5],
        ]);
    });
});

describe('the address', () => {
    it('should keep the lanes it knows, as a list or as the bar writes them, and a zoom with both ends', () => {
        expect(parseStatisticsSearch({ lanes: ['ping', 'nope', 'disk'] }).lanes).toEqual(['ping', 'disk']);
        expect(parseStatisticsSearch({ lanes: 'disk,heap' }).lanes).toEqual(['heap', 'disk']);
        expect(parseStatisticsSearch({ from: 100 })).toEqual(
            expect.objectContaining({ from: undefined, to: undefined }),
        );
        expect(parseStatisticsSearch({ tab: 'insights', range: '7d', insights: '2d' })).toEqual(
            expect.objectContaining({ tab: 'insights', range: '7d', insights: undefined }),
        );
    });

    it('should read the zoom, or the range ending now', () => {
        expect(timelineWindow({ from: 10, to: 70 }, 1000)).toEqual({ from: 10, to: 70, zoomed: true });
        expect(timelineWindow({ range: '1h' }, 10_000)).toEqual({ from: 6400, to: 10_000, zoomed: false });
        expect(timelineWindow({}, 100_000)).toEqual({ from: 13_600, to: 100_000, zoomed: false });
    });
});
