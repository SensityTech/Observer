import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import {
    Activity,
    ArrowRight,
    Cpu,
    Gauge,
    MemoryStick,
    Server,
    TriangleAlert,
    Unplug,
    Users,
    type LucideIcon,
} from 'lucide-react';
import { use, useMemo, type ReactNode } from 'react';

import { summaryQuery } from '../api/drops';
import { Permission } from '../api/generated/Permission';
import { healthQuery } from '../api/health';
import type { LiveSample } from '../api/generated/LiveSample';
import type { MetricsSeries } from '../api/generated/MetricsSeries';
import type { RunGrowth } from '../api/generated/RunGrowth';
import type { Series } from '../api/generated/Series';
import { growthQuery, seriesQuery } from '../api/metrics';
import { allows, MeContext } from '../auth/me';
import { Chip } from '../components/Chip';
import { Distribution } from '../components/Distribution';
import { Sparkline } from '../components/Sparkline';
import { formatRate, trendWindows, VerdictChip } from '../drops/words';
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
    'fxsKernel',
    'fxsPrivate',
    'hostCpu',
    'observerWorkingSet',
] as const satisfies readonly Series[];

type HourSeries = (typeof TILE_SERIES)[number];

/** Under this share of its maximum the processors' clock reads as held back. */
const LIMITED_BELOW = 95;

/** A figure split into its number and its unit, as the tile shows them: `9.4` and `GB`. */
export const split = (formatted: string): [string, string] => {
    const space = formatted.lastIndexOf(' ');
    return space < 0 ? [formatted, ''] : [formatted.slice(0, space), formatted.slice(space + 1)];
};

export const total = (values: readonly (number | null)[]) =>
    values.some((value) => value !== null)
        ? values.reduce<number>((sum, value) => sum + (value ?? 0), 0)
        : null;

/**
 * The six tiles of the Overview: the latest sample, and the last hour for the trends; then the
 * players' drops of the last 24 hours, for who sees them.
 */
export function Tiles() {
    const { recent, latest } = useLiveMetrics();
    const hour = useQuery(seriesQuery('1h', TILE_SERIES));
    const growth = useQuery(growthQuery);
    const merged = useMemo(() => (hour.data ? mergeLive(hour.data, recent) : null), [hour.data, recent]);
    const trend = (series: HourSeries) => merged?.series.find((data) => data.series === series)?.values ?? [];

    return (
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-3">
            <PlayersTile latest={latest} trend={trend} />
            <TickTile latest={latest} trend={trend} hour={merged} />
            <CpuTile latest={latest} trend={trend} />
            <MemoryTile latest={latest} trend={trend} growth={growth.data ?? null} />
            <HostTile latest={latest} trend={trend} />
            <ObserverTile latest={latest} trend={trend} />
            <DropsTile />
        </div>
    );
}

type TileProps = {
    latest: LiveSample | null;
    trend: (series: HourSeries) => readonly (number | null)[];
};

/** A fact of a tile: its term, its value, and whether it is the one that warns. */
export type Fact = { term: string; detail: ReactNode; warning?: boolean };

export function Facts({ facts }: { facts: readonly Fact[] }) {
    return (
        <dl className="flex flex-col border-t border-line">
            {facts.map(({ term, detail, warning }) => (
                <div
                    key={term}
                    className="flex items-baseline justify-between gap-3 border-b border-line pt-1.5 pb-1.25 text-small last:border-b-0"
                >
                    <dt className="text-ink-3">{term}</dt>
                    <dd className={cn('text-right', warning ? 'text-warning' : 'text-ink-2')}>{detail}</dd>
                </div>
            ))}
        </dl>
    );
}

export function Tile({
    icon: Icon,
    label,
    aside,
    value,
    unit,
    warning = false,
    facts,
    trend,
}: {
    icon: LucideIcon;
    label: string;
    aside?: ReactNode;
    value: string;
    unit?: string;
    warning?: boolean;
    facts: Fact[];
    trend: ReactNode;
}) {
    // A percentage keeps its sign against the number: `182%` then ` of one core`.
    const glued = value.endsWith('%');
    return (
        <section
            className="row-span-4 grid min-w-0 grid-rows-subgrid items-start gap-y-2.5 rounded-card border border-line bg-surface px-4 pt-3.75 pb-3.5"
            aria-label={label}
        >
            <h2 className="flex min-h-5.5 flex-wrap items-center gap-x-1.5 gap-y-1 text-small text-ink-3">
                <Icon size={14} strokeWidth={1.75} aria-hidden="true" />
                {label}
                {aside}
            </h2>
            <p className={cn('text-tile whitespace-nowrap', warning ? 'text-warning' : 'text-ink')}>
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
            <Facts facts={facts} />
            {trend}
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
            unit={slots === null ? undefined : t.tiles.players.of(format.number(slots))}
            facts={[
                {
                    term: t.tiles.players.ping,
                    detail:
                        average === null || p95 === null
                            ? t.tiles.unknown
                            : t.tiles.players.pings(format.milliseconds(average), format.milliseconds(p95)),
                },
                {
                    term: t.tiles.players.lastHour,
                    detail:
                        joined === null || left === null
                            ? t.tiles.unknown
                            : t.tiles.players.joinedLeft(format.number(joined), format.number(left)),
                },
            ]}
            trend={<Sparkline className="self-end" values={trend('players')} label={t.tiles.players.trend} />}
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
                { term: t.tiles.tick.overBudget, detail: shown('mainOverBudget', format.percent) },
                {
                    term: t.tiles.tick.others,
                    detail:
                        network === null && sync === null
                            ? t.tiles.unknown
                            : t.tiles.pair(
                                  network === null ? t.tiles.unknown : format.milliseconds(network),
                                  sync === null ? t.tiles.unknown : format.milliseconds(sync),
                              ),
                },
                { term: t.tiles.tick.lastHitch, detail: hitch },
            ]}
            trend={<Sparkline className="self-end" values={trend('mainTick')} label={t.tiles.tick.trend} />}
        />
    );
}

/**
 * The kernel's part of FXServer's CPU over the slots that measured both, `null` at 0. One slot is
 * too short to split: Windows counts CPU in 15.6 ms ticks, a few of them at a low load.
 */
const kernelPart = (cpu: readonly (number | null)[], kernel: readonly (number | null)[]) => {
    let used = 0;
    let inKernel = 0;
    cpu.forEach((value, index) => {
        const part = kernel[index] ?? null;
        if (value !== null && part !== null) {
            used += value;
            inKernel += part;
        }
    });
    return used > 0 ? Math.min(1, inKernel / used) : null;
};

function CpuTile({ latest, trend }: TileProps) {
    const { t, format, value, shown } = useValues(latest);
    const cores = latest?.cores ?? null;
    const cpu = value('fxsCpu');
    const share = (series: Series) => shown(series, format.percent);
    // Over the last hour, while the server runs.
    const kernelShare = cpu === null ? null : kernelPart(trend('fxsCpu'), trend('fxsKernel'));
    return (
        <Tile
            icon={Cpu}
            label={t.tiles.cpu.label}
            value={shown('fxsCpu', format.percent)}
            unit={cpu === null ? undefined : t.tiles.cpu.unit}
            facts={[
                {
                    term: t.tiles.cpu.machine,
                    detail:
                        cores === null
                            ? t.tiles.unknown
                            : shown('fxsCpu', (used) => format.percent(used / cores)),
                },
                { term: t.tiles.cpu.main, detail: share('mainThreadCpu') },
                {
                    term: t.tiles.cpu.networkSync,
                    detail: t.tiles.pair(share('networkThreadCpu'), share('syncThreadCpu')),
                },
                {
                    term: t.tiles.cpu.runtimes,
                    detail: t.tiles.triple(
                        share('jsThreadsCpu'),
                        share('monoThreadsCpu'),
                        share('otherThreadsCpu'),
                    ),
                },
            ]}
            trend={
                <Distribution
                    className="self-end"
                    parts={[
                        {
                            name: t.tiles.cpu.user,
                            share: kernelShare === null ? null : 1 - kernelShare,
                            fill: 'bg-ink-2',
                        },
                        { name: t.tiles.cpu.kernel, share: kernelShare, fill: 'bg-chart-muted' },
                    ]}
                />
            }
        />
    );
}

/** A count's growth, whole and signed: `+12`, and `0` once it rounds to nothing either way. */
export const GROWTH = {
    maximumFractionDigits: 0,
    signDisplay: 'exceptZero',
} satisfies Intl.NumberFormatOptions;

function MemoryTile({ latest, trend, growth }: TileProps & { growth: RunGrowth | null }) {
    const { t, format, value } = useValues(latest);
    const privateBytes = value('fxsPrivate');
    const [number, unit] = privateBytes === null ? [t.tiles.unknown, ''] : split(format.bytes(privateBytes));
    const workingSet = value('fxsWorkingSet');
    const peak = value('fxsWorkingSetPeak');
    const used = value('jsHeapUsed');
    const heap = value('jsHeapTotal');
    const read = value('fxsRead');
    const written = value('fxsWrite');
    const rate = (bytes: number | null) =>
        bytes === null ? t.tiles.unknown : t.tiles.memory.perSecond(format.bytes(bytes));
    const memory = growth?.private ?? null;
    const handles = growth?.handles ?? null;
    const leaking = memory?.leak === true;
    const facts: Fact[] = [
        {
            term: t.tiles.memory.sinceStart,
            detail:
                memory === null
                    ? t.tiles.unknown
                    : t.tiles.memory.perHour(format.bytes(memory.perHour, { signed: true })),
            warning: leaking,
        },
        {
            term: t.tiles.memory.workingSet,
            detail:
                workingSet === null
                    ? t.tiles.unknown
                    : peak === null
                      ? format.bytes(workingSet)
                      : t.tiles.memory.withPeak(format.bytes(workingSet), format.bytes(peak)),
        },
        {
            term: t.tiles.memory.heap,
            detail:
                used === null || heap === null
                    ? t.tiles.unknown
                    : t.tiles.ratio(format.bytes(used), format.bytes(heap)),
        },
        {
            term: t.tiles.memory.io,
            detail:
                read === null && written === null ? t.tiles.unknown : t.tiles.pair(rate(read), rate(written)),
        },
    ];
    if (handles?.leak === true) {
        facts.push({
            term: t.tiles.memory.handles,
            detail: t.tiles.memory.handlesSinceStart(format.number(handles.perHour, GROWTH)),
            warning: true,
        });
    }
    return (
        <Tile
            icon={MemoryStick}
            label={t.tiles.memory.label}
            aside={
                (leaking || handles?.leak === true) && (
                    <Chip tone="warning" icon={TriangleAlert} className="ml-auto">
                        {t.tiles.memory.possibleLeak}
                    </Chip>
                )
            }
            value={number}
            unit={unit === '' ? undefined : t.tiles.memory.unit(unit)}
            warning={leaking}
            facts={facts}
            trend={
                <Sparkline
                    className="self-end"
                    values={trend('fxsPrivate')}
                    label={t.tiles.memory.trend}
                    warning={leaking}
                />
            }
        />
    );
}

function HostTile({ latest, trend }: TileProps) {
    const { t, format, value, shown } = useValues(latest);
    const used = value('hostMemoryUsed');
    const memory = value('hostMemoryTotal');
    const free = value('hostDiskFree');
    const latency = value('hostDiskLatency');
    const transfers = value('hostDiskTransfers');
    const received = value('hostReceive');
    const sent = value('hostSend');
    const clock = value('hostClock');
    const clockMax = value('hostClockMax');
    const limit = value('hostClockLimit');
    const steal = value('hostSteal');
    const pressure = (['hostPressureCpu', 'hostPressureMemory', 'hostPressureIo'] as const).map(value);
    const ghz = (mhz: number) =>
        format.number(mhz / 1000, { minimumFractionDigits: 1, maximumFractionDigits: 1 });

    let diskLatency: string = t.tiles.unknown;
    if (transfers === 0) {
        diskLatency = t.tiles.host.idle;
    } else if (latency !== null) {
        diskLatency = format.milliseconds(latency);
    }
    const facts: Fact[] = [
        {
            term: t.tiles.host.memory,
            detail:
                used === null || memory === null
                    ? t.tiles.unknown
                    : t.tiles.ratio(format.bytes(used), format.bytes(memory)),
        },
        {
            term: t.tiles.host.disk,
            detail: t.tiles.pair(free === null ? t.tiles.unknown : format.bytes(free), diskLatency),
        },
        {
            term: t.tiles.host.network,
            detail:
                received === null || sent === null
                    ? t.tiles.unknown
                    : t.tiles.pair(format.bitRate(received), format.bitRate(sent)),
        },
        {
            term: t.tiles.host.clock,
            detail:
                clock === null || clockMax === null ? (
                    t.tiles.unknown
                ) : (
                    <>
                        {t.tiles.host.clockValue(ghz(clock), ghz(clockMax))}
                        {limit !== null && limit < LIMITED_BELOW && (
                            <span className="text-warning">
                                {t.tiles.host.limited(format.percent(limit))}
                            </span>
                        )}
                    </>
                ),
        },
    ];
    // Only Linux says these; Windows never shows the rows.
    if (steal !== null) {
        facts.push({ term: t.tiles.host.steal, detail: format.percent(steal) });
    }
    if (pressure.some((share) => share !== null)) {
        const [cpu, ram, io] = pressure.map((share) =>
            share === null ? t.tiles.unknown : format.percent(share),
        );
        facts.push({
            term: t.tiles.host.pressure,
            detail: t.tiles.host.pressureValue(cpu ?? '', ram ?? '', io ?? ''),
        });
    }
    return (
        <Tile
            icon={Server}
            label={t.tiles.host.label}
            value={shown('hostCpu', format.percent)}
            unit={value('hostCpu') === null ? undefined : t.tiles.host.unit}
            facts={facts}
            trend={<Sparkline className="self-end" values={trend('hostCpu')} label={t.tiles.host.trend} />}
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
                { term: t.tiles.observer.cpu, detail: shown('observerCpu', format.percent) },
                { term: t.tiles.observer.clients, detail: shown('dashboardClients', format.number) },
                {
                    term: t.tiles.observer.databases,
                    detail:
                        state === null || metrics === null
                            ? t.tiles.unknown
                            : t.tiles.observer.databasesValue(format.bytes(state), format.bytes(metrics)),
                },
                {
                    term: t.tiles.observer.version,
                    detail:
                        version === null
                            ? t.tiles.unknown
                            : status === null
                              ? version
                              : t.tiles.observer.versionValue(
                                    version,
                                    format.duration(secondsSince(status.startedAt, now)),
                                ),
                },
            ]}
            trend={
                <Sparkline
                    className="self-end"
                    values={trend('observerWorkingSet')}
                    label={t.tiles.observer.trend}
                />
            }
        />
    );
}

/**
 * Unexpected drops per 100 player-hours over the last 24 hours, today against yesterday's same
 * hours, and the way to the page: for who sees drop statistics, and read only for them.
 */
function DropsTile() {
    const me = use(MeContext);
    return me !== null && allows(me, Permission.DropsView) ? <DropsTileShown /> : null;
}

function DropsTileShown() {
    const { t, locale, format } = useI18n();
    const words = t.drops.overview;
    const summary = useQuery(summaryQuery('24h'));
    const data = summary.data;
    const tile = data?.tiles.find((each) => each.kind === 'unexpected');
    const rising = tile?.trend.verdict === 'rising';
    return (
        <Tile
            icon={Unplug}
            label={words.label}
            aside={
                data !== undefined &&
                tile !== undefined && (
                    <VerdictChip
                        trend={tile.trend}
                        windows={trendWindows(t, locale, data)}
                        className="ml-auto"
                    />
                )
            }
            value={tile?.rate == null ? t.drops.unknown : formatRate(locale, tile.rate)}
            unit={tile?.rate == null ? undefined : t.drops.unit}
            warning={rising}
            facts={[
                {
                    term: words.ended,
                    detail: tile?.oneIn == null ? t.drops.unknown : t.drops.tiles.oneIn(tile.oneIn),
                },
                {
                    term: words.drops,
                    detail: tile === undefined ? t.drops.unknown : format.number(tile.drops),
                },
            ]}
            trend={
                <div className="flex flex-col gap-2 self-end">
                    <Sparkline values={tile?.spark ?? []} label={words.trend} warning={rising} />
                    <Link
                        to="/drops"
                        search={{ range: '24h' }}
                        className="inline-flex items-center gap-1.5 self-start rounded-xs text-small font-medium text-ink-2 underline-offset-2 hover:text-ink hover:underline max-md:min-h-11"
                    >
                        {words.open}
                        <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                    </Link>
                </div>
            }
        />
    );
}
