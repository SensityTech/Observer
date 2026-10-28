import { useQuery } from '@tanstack/react-query';
import { Cpu, Gauge, MemoryStick, Server, Users, Activity, type LucideIcon } from 'lucide-react';
import { useMemo, type ReactNode } from 'react';

import { healthQuery } from '../api/health';
import type { LiveSample } from '../api/generated/LiveSample';
import type { MetricsSeries } from '../api/generated/MetricsSeries';
import type { Series } from '../api/generated/Series';
import { seriesQuery } from '../api/metrics';
import { Sparkline } from '../components/Sparkline';
import { useI18n } from '../i18n/I18nProvider';
import { useSnapshot } from '../live/context';
import { cn } from '../lib/cn';
import { secondsSince, useNow } from '../lib/useNow';
import { useLiveMetrics } from '../metrics/live';
import { mergeLive } from '../metrics/merge';

/** The last hour behind the tiles' trends and facts. */
export const TILE_SERIES = [
    'players',
    'slots',
    'joins',
    'leaves',
    'mainTick',
    'hitchesMain',
    'hitchMax',
    'fxsCpu',
    'fxsPrivate',
    'hostCpu',
    'observerWorkingSet',
] as const satisfies readonly Series[];

type HourSeries = (typeof TILE_SERIES)[number];

/** A figure split into its number and its unit, as the tile shows them: `9.4` and `GB`. */
const split = (formatted: string): [string, string] => {
    const space = formatted.lastIndexOf(' ');
    return space < 0 ? [formatted, ''] : [formatted.slice(0, space), formatted.slice(space + 1)];
};

const total = (values: readonly (number | null)[]) =>
    values.some((value) => value !== null)
        ? values.reduce<number>((sum, value) => sum + (value ?? 0), 0)
        : null;

/** The six tiles of the Overview: the latest sample, and the last hour for the trends. */
export function Tiles() {
    const { recent, latest } = useLiveMetrics();
    const hour = useQuery(seriesQuery('1h', TILE_SERIES));
    const merged = useMemo(() => (hour.data ? mergeLive(hour.data, recent) : null), [hour.data, recent]);
    const trend = (series: HourSeries) => merged?.series.find((data) => data.series === series)?.values ?? [];

    return (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            <PlayersTile latest={latest} trend={trend} />
            <TickTile latest={latest} trend={trend} hour={merged} />
            <CpuTile latest={latest} trend={trend} />
            <MemoryTile latest={latest} trend={trend} />
            <HostTile latest={latest} trend={trend} />
            <ObserverTile latest={latest} trend={trend} />
        </div>
    );
}

type TileProps = {
    latest: LiveSample | null;
    trend: (series: HourSeries) => readonly (number | null)[];
};

function Tile({
    icon: Icon,
    label,
    aside,
    value,
    unit,
    facts,
    trend,
    trendLabel,
}: {
    icon: LucideIcon;
    label: string;
    aside?: ReactNode;
    value: string;
    unit?: string;
    facts: [string, ReactNode][];
    trend: readonly (number | null)[];
    trendLabel: string;
}) {
    // A percentage keeps its sign against the number: `182%` then ` of one core`.
    const glued = value.endsWith('%');
    return (
        <section
            className="row-span-4 grid min-w-0 grid-rows-subgrid items-start gap-y-2.5 rounded-card border border-line bg-surface px-4 pt-3.75 pb-3.5"
            aria-label={label}
        >
            <h2 className="flex min-h-5.5 items-center gap-1.5 text-small text-ink-3">
                <Icon size={14} strokeWidth={1.75} aria-hidden="true" />
                {label}
                {aside}
            </h2>
            <p className="text-tile whitespace-nowrap text-ink">
                {value}
                {unit && (
                    <span
                        className={cn(
                            'text-body font-medium tracking-normal text-ink-3',
                            !glued && 'ml-1.25',
                        )}
                    >
                        {glued ? ` ${unit}` : unit}
                    </span>
                )}
            </p>
            <dl className="flex flex-col border-t border-line">
                {facts.map(([term, detail]) => (
                    <div
                        key={term}
                        className="flex items-baseline justify-between gap-3 border-b border-line pt-1.5 pb-1.25 text-small last:border-b-0"
                    >
                        <dt className="text-ink-3">{term}</dt>
                        <dd className="text-right text-ink-2">{detail}</dd>
                    </div>
                ))}
            </dl>
            <Sparkline className="self-end" values={trend} label={trendLabel} />
        </section>
    );
}

function useValues(latest: LiveSample | null) {
    const { t, format } = useI18n();
    const value = (series: Series) => latest?.values[series] ?? null;
    const shown = (series: Series, as: (value: number) => string) => {
        const measured = value(series);
        return measured === null ? t.tiles.unknown : as(measured);
    };
    return { t, format, value, shown };
}

function PlayersTile({ latest, trend }: TileProps) {
    const { t, format, value, shown } = useValues(latest);
    const slots = value('slots');
    const joined = total(trend('joins'));
    const left = total(trend('leaves'));
    const average = value('pingAvg');
    const p95 = value('pingP95');
    return (
        <Tile
            icon={Users}
            label={t.tiles.players.label}
            value={shown('players', format.number)}
            unit={slots === null ? undefined : `/ ${format.number(slots)}`}
            facts={[
                [
                    t.tiles.players.ping,
                    average === null || p95 === null
                        ? t.tiles.unknown
                        : t.tiles.players.pings(format.milliseconds(average), format.milliseconds(p95)),
                ],
                [
                    t.tiles.players.lastHour,
                    joined === null || left === null
                        ? t.tiles.unknown
                        : t.tiles.players.joinedLeft(format.number(joined), format.number(left)),
                ],
            ]}
            trend={trend('players')}
            trendLabel={t.tiles.players.trend}
        />
    );
}

function TickTile({ latest, trend, hour }: TileProps & { hour: MetricsSeries | null }) {
    const { t, format, value, shown } = useValues(latest);
    const network = value('networkTick');
    const sync = value('syncTick');
    const hitches = trend('hitchMax');
    const last = hitches.findLastIndex((ms) => ms !== null);
    // A slot without a hitch has no longest one, nor has a slot nothing measured: only the count
    // tells that svMain was followed and did not hitch.
    const followed = trend('hitchesMain').some((count) => count !== null);
    let hitch: string = t.tiles.unknown;
    if (hour !== null && last >= 0) {
        const at = new Date((hour.from + last * hour.step) * 1000);
        hitch = t.tiles.tick.hitchAt(
            format.milliseconds(hitches[last] ?? 0),
            format.dateTime(at, { dateStyle: undefined }),
        );
    } else if (followed) {
        hitch = t.tiles.tick.noHitch;
    }
    return (
        <Tile
            icon={Activity}
            label={t.tiles.tick.label}
            value={shown('mainTick', (ms) => split(format.milliseconds(ms))[0])}
            unit={value('mainTick') === null ? undefined : t.tiles.tick.unit}
            facts={[
                [t.tiles.tick.overBudget, shown('mainOverBudget', format.percent)],
                [
                    t.tiles.tick.others,
                    network === null && sync === null
                        ? t.tiles.unknown
                        : t.tiles.tick.pair(
                              network === null ? t.tiles.unknown : format.milliseconds(network),
                              sync === null ? t.tiles.unknown : format.milliseconds(sync),
                          ),
                ],
                [t.tiles.tick.lastHitch, hitch],
            ]}
            trend={trend('mainTick')}
            trendLabel={t.tiles.tick.trend}
        />
    );
}

function CpuTile({ latest, trend }: TileProps) {
    const { t, format, value, shown } = useValues(latest);
    const threads = value('fxsThreads');
    const handles = value('fxsHandles');
    const children = value('children');
    const childrenCpu = value('childrenCpu');
    const cores = latest?.cores ?? null;
    return (
        <Tile
            icon={Cpu}
            label={t.tiles.cpu.label}
            value={shown('fxsCpu', format.percent)}
            unit={value('fxsCpu') === null ? undefined : t.tiles.cpu.unit}
            facts={[
                [
                    t.tiles.cpu.machine,
                    cores === null ? t.tiles.unknown : shown('fxsCpu', (cpu) => format.percent(cpu / cores)),
                ],
                [
                    t.tiles.cpu.threadsHandles,
                    t.tiles.tick.pair(
                        threads === null ? t.tiles.unknown : format.number(threads),
                        handles === null ? t.tiles.unknown : format.number(handles),
                    ),
                ],
                [
                    t.tiles.cpu.children,
                    children === null
                        ? t.tiles.unknown
                        : t.tiles.cpu.childrenValue(
                              children,
                              childrenCpu === null ? t.tiles.unknown : format.percent(childrenCpu),
                          ),
                ],
            ]}
            trend={trend('fxsCpu')}
            trendLabel={t.tiles.cpu.trend}
        />
    );
}

function MemoryTile({ latest, trend }: TileProps) {
    const { t, format, value, shown } = useValues(latest);
    const privateBytes = value('fxsPrivate');
    const [number, unit] = privateBytes === null ? [t.tiles.unknown, ''] : split(format.bytes(privateBytes));
    const used = value('jsHeapUsed');
    const heap = value('jsHeapTotal');
    const read = value('fxsRead');
    const written = value('fxsWrite');
    const rate = (bytes: number | null) =>
        bytes === null ? t.tiles.unknown : t.tiles.memory.perSecond(format.bytes(bytes));
    return (
        <Tile
            icon={MemoryStick}
            label={t.tiles.memory.label}
            value={number}
            unit={unit === '' ? undefined : t.tiles.memory.unit(unit)}
            facts={[
                [t.tiles.memory.workingSet, shown('fxsWorkingSet', format.bytes)],
                [
                    t.tiles.memory.heap,
                    used === null || heap === null
                        ? t.tiles.unknown
                        : t.tiles.ratio(format.bytes(used), format.bytes(heap)),
                ],
                [
                    t.tiles.memory.io,
                    read === null && written === null
                        ? t.tiles.unknown
                        : t.tiles.tick.pair(rate(read), rate(written)),
                ],
            ]}
            trend={trend('fxsPrivate')}
            trendLabel={t.tiles.memory.trend}
        />
    );
}

function HostTile({ latest, trend }: TileProps) {
    const { t, format, value, shown } = useValues(latest);
    const used = value('hostMemoryUsed');
    const memory = value('hostMemoryTotal');
    const free = value('hostDiskFree');
    const disk = value('hostDiskTotal');
    const received = value('hostReceive');
    const sent = value('hostSend');
    return (
        <Tile
            icon={Server}
            label={t.tiles.host.label}
            value={shown('hostCpu', format.percent)}
            unit={value('hostCpu') === null ? undefined : t.tiles.host.unit}
            facts={[
                [
                    t.tiles.host.memory,
                    used === null || memory === null
                        ? t.tiles.unknown
                        : t.tiles.ratio(format.bytes(used), format.bytes(memory)),
                ],
                [
                    t.tiles.host.disk,
                    free === null || disk === null
                        ? t.tiles.unknown
                        : t.tiles.ratio(format.bytes(free), format.bytes(disk)),
                ],
                [
                    t.tiles.host.network,
                    received === null || sent === null
                        ? t.tiles.unknown
                        : t.tiles.tick.pair(format.bitRate(received), format.bitRate(sent)),
                ],
            ]}
            trend={trend('hostCpu')}
            trendLabel={t.tiles.host.trend}
        />
    );
}

function ObserverTile({ latest, trend }: TileProps) {
    const { t, format, value, shown } = useValues(latest);
    const health = useQuery(healthQuery);
    const status = useSnapshot('status');
    const now = useNow();
    const memory = value('observerWorkingSet');
    const [number, unit] = memory === null ? [t.tiles.unknown, ''] : split(format.bytes(memory));
    const state = value('stateDatabase');
    const metrics = value('metricsDatabase');
    const reachable = !health.isError && health.data !== undefined;
    const version = health.data?.version ?? status?.version ?? null;
    return (
        <Tile
            icon={Gauge}
            label={t.tiles.observer.label}
            aside={
                health.isPending ? undefined : (
                    <span className="ml-auto flex items-center gap-1.5 text-ink-2">
                        <span className={cn('led', reachable ? 'led-live' : 'led-down')} aria-hidden="true" />
                        {reachable ? t.tiles.observer.running : t.tiles.observer.unreachable}
                    </span>
                )
            }
            value={number}
            unit={unit === '' ? undefined : t.tiles.observer.unit(unit)}
            facts={[
                [t.tiles.observer.cpu, shown('observerCpu', format.percent)],
                [t.tiles.observer.clients, shown('dashboardClients', format.number)],
                [
                    t.tiles.observer.databases,
                    state === null || metrics === null
                        ? t.tiles.unknown
                        : t.tiles.observer.databasesValue(format.bytes(state), format.bytes(metrics)),
                ],
                [
                    t.tiles.observer.version,
                    version === null
                        ? t.tiles.unknown
                        : status === null
                          ? version
                          : t.tiles.observer.versionValue(
                                version,
                                format.duration(secondsSince(status.startedAt, now)),
                            ),
                ],
            ]}
            trend={trend('observerWorkingSet')}
            trendLabel={t.tiles.observer.trend}
        />
    );
}
