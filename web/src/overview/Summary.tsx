import { useQuery } from '@tanstack/react-query';
import { TriangleAlert } from 'lucide-react';

import { growthQuery, seriesQuery } from '../api/metrics';
import { formatClock } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useLiveMetrics } from '../metrics/live';
import { useSchedule } from '../schedule/context';
import { useServerStatus } from '../server/context';
import { split } from './Tiles';

/** The day behind the players' peak, which the Recent card reads its markers from too. */
export const DAY = seriesQuery('24h', ['players']);

/** svMain's budget per tick, which the tick tile says the share of ticks over. */
const MAIN_BUDGET_MS = 50;

/** Four figures of the server now, each with one line that puts it in context. */
export function Summary() {
    const { t, locale, format } = useI18n();
    const words = t.overview.tiles;
    const { latest } = useLiveMetrics();
    const { status } = useServerStatus();
    const { zone } = useSchedule();
    const day = useQuery(DAY);
    const growth = useQuery(growthQuery);
    const value = (series: 'players' | 'slots' | 'mainTick' | 'mainOverBudget' | 'fxsCpu' | 'fxsPrivate') =>
        latest?.values[series] ?? null;

    const players = value('players') ?? status?.players?.online ?? null;
    const slots = value('slots') ?? status?.players?.max ?? null;
    let peak: string = words.unknown;
    if (day.data !== undefined) {
        const values = day.data.series.find((each) => each.series === 'players')?.values ?? [];
        let top = -1;
        values.forEach((count, index) => {
            if (count !== null && (top < 0 || count > (values[top] ?? 0))) {
                top = index;
            }
        });
        const most = values[top] ?? null;
        if (most === null || most === 0) {
            peak = words.noPeak;
        } else {
            const at = (day.data.from + top * day.data.step) * 1000;
            peak = words.peak(format.number(Math.round(most)), formatClock(locale, at, zone ?? 'UTC'));
        }
    }

    const tick = value('mainTick');
    const over = value('mainOverBudget');
    const cpu = value('fxsCpu');
    const cores = latest?.cores ?? null;
    const memory = value('fxsPrivate');
    const [memoryValue, memoryUnit] = memory === null ? [words.unknown, ''] : split(format.bytes(memory));
    const rise = growth.data?.private ?? null;
    const leaking = rise?.leak === true;
    let memoryLine: string = words.unknown;
    if (rise !== null) {
        const rate = format.bytes(rise.perHour, { signed: true });
        memoryLine = leaking ? words.possibleLeak(rate) : words.growing(rate);
    }

    return (
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <Figure
                label={words.players}
                value={players === null ? words.unknown : format.number(players)}
                unit={slots === null ? undefined : words.of(format.number(slots))}
                line={peak}
            />
            <Figure
                label={words.tick}
                value={tick === null ? words.unknown : split(format.milliseconds(tick))[0]}
                unit={tick === null ? undefined : split(format.milliseconds(tick))[1]}
                line={
                    over === null
                        ? words.unknown
                        : words.overBudget(format.percent(over), format.milliseconds(MAIN_BUDGET_MS))
                }
            />
            <Figure
                label={words.cpu}
                value={cpu === null ? words.unknown : format.percent(cores === null ? cpu : cpu / cores)}
                line={cpu === null ? words.unknown : cores === null ? words.ofOneCore : words.ofCores(cores)}
            />
            <Figure
                label={words.memory}
                value={memoryValue}
                unit={memoryUnit === '' ? undefined : memoryUnit}
                line={memoryLine}
                warning={leaking}
            />
        </div>
    );
}

function Figure({
    label,
    value,
    unit,
    line,
    warning = false,
}: {
    label: string;
    value: string;
    unit?: string;
    line: string;
    warning?: boolean;
}) {
    return (
        <section className="card flex flex-col gap-1.5 px-4 pt-3.5 pb-3" aria-label={label}>
            <h2 className="eyebrow">{label}</h2>
            <p
                className={cn(
                    'text-figure whitespace-nowrap tabular-nums',
                    warning ? 'text-warning' : 'text-ink',
                )}
            >
                {value}
                {unit !== undefined && (
                    <span className="ml-1.25 text-body font-normal tracking-normal text-ink-3">{unit}</span>
                )}
            </p>
            <p className={cn('flex items-start gap-1.5 text-small', warning ? 'text-warning' : 'text-ink-3')}>
                {warning && (
                    <TriangleAlert
                        size={14}
                        strokeWidth={1.75}
                        className="mt-0.5 shrink-0"
                        aria-hidden="true"
                    />
                )}
                {line}
            </p>
        </section>
    );
}
