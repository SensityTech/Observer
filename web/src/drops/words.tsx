import { CircleDashed, Minus, TrendingDown, TrendingUp, type LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';

import type { DropCategory } from '../api/generated/DropCategory';
import type { DropsSummary } from '../api/generated/DropsSummary';
import type { Trend } from '../api/generated/Trend';
import type { TrendVerdict } from '../api/generated/TrendVerdict';
import { chipVariants } from '../components/Chip';
import { Toggletip } from '../components/Toggletip';
import type { Messages } from '../i18n/en';
import { FIGURE, formatClock, formatDay, formatNumber } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { DROP_ICONS } from '../players/words';

export const VERDICT_ICONS = {
    rising: TrendingUp,
    falling: TrendingDown,
    noClearChange: Minus,
    notEnoughDrops: CircleDashed,
} satisfies Record<TrendVerdict, LucideIcon>;

/** A rise is the one verdict that warns: the others are information, in ink. */
export const verdictTone = (verdict: TrendVerdict) => (verdict === 'rising' ? 'warning' : 'neutral');

/** A change as the page writes it: `+38%`, `−20%` with the minus sign, `0%`. */
export const signedPercent = (locale: string, change: number) =>
    formatNumber(locale, change, {
        style: 'percent',
        maximumFractionDigits: 0,
        signDisplay: 'exceptZero',
    }).replace('-', '−');

/** A rate per 100 player-hours, three significant digits at most: `4.6`, `12`, `0.4`. */
export const formatRate = (locale: string, rate: number) => formatNumber(locale, rate, FIGURE);

/** A share from 0 to 1 as a percentage: `41%`. */
export const formatShare = (locale: string, share: number) =>
    formatNumber(locale, share, { ...FIGURE, style: 'percent' });

/**
 * The day from which the figures that count players read sessions, `3 Oct`, when it comes after
 * `from`: sessions older than the retention are gone. `null` when the whole window is counted.
 */
export const keptSince = (locale: string, keptFrom: string, from: string | undefined, zone: string) =>
    from !== undefined && Date.parse(keptFrom) > Date.parse(from)
        ? formatDay(locale, keptFrom, zone, { weekday: false })
        : null;

/** What the chip says: the verdict, with its change once the test called it. */
export const verdictText = (t: Messages, locale: string, trend: Trend) => {
    const word = t.drops.verdicts[trend.verdict];
    const called = trend.verdict === 'rising' || trend.verdict === 'falling';
    return called && trend.change !== null ? `${word} ${signedPercent(locale, trend.change)}` : word;
};

/** The chip's description: the change and its interval, both rates, the drops counted, the windows. */
export const verdictDetails = (t: Messages, locale: string, trend: Trend, windows: string) => {
    const words = t.drops.verdict;
    const percent = (value: number) => signedPercent(locale, value);
    const rate = (value: number | null) => (value === null ? t.drops.unknown : formatRate(locale, value));
    const lines: string[] = [];
    if (trend.change !== null) {
        lines.push(
            trend.low !== null && trend.high !== null
                ? words.interval(percent(trend.change), percent(trend.low), percent(trend.high))
                : percent(trend.change),
        );
    }
    lines.push(words.rates(rate(trend.now), rate(trend.before)));
    lines.push(words.events(trend.eventsNow, trend.eventsBefore));
    if (trend.verdict === 'notEnoughDrops') {
        lines.push(words.notEnough);
    } else if (trend.verdict === 'noClearChange') {
        lines.push(words.noClearChange);
    }
    lines.push(windows);
    return lines;
};

/** Which windows a summary's trends compare, in words: a day's are today and yesterday's same hours. */
export const trendWindows = (
    t: Messages,
    locale: string,
    summary: Pick<DropsSummary, 'range' | 'zone' | 'trendFrom' | 'beforeFrom' | 'beforeTo'>,
) => {
    if (summary.range !== '24h') {
        return t.drops.windows[summary.range];
    }
    const clock = (instant: string) => formatClock(locale, instant, summary.zone);
    return t.drops.windows.today(
        clock(summary.trendFrom),
        clock(summary.beforeFrom),
        clock(summary.beforeTo),
    );
};

/**
 * The verdict as a chip: an icon and a word, never a colour alone, `Rising +38%` in warning. Its
 * interval, both rates, the events the test counted and the windows compared are its description,
 * shown on hover and pinned by a click.
 */
export function VerdictChip({
    trend,
    windows,
    className,
}: {
    trend: Trend;
    /** Which windows were compared, said last in the description. */
    windows: string;
    className?: string;
}) {
    const { t, locale } = useI18n();
    const Icon = VERDICT_ICONS[trend.verdict];
    const lines = verdictDetails(t, locale, trend, windows);
    return (
        <Toggletip
            className={cn(chipVariants({ tone: verdictTone(trend.verdict) }), className)}
            tip={lines.map((line, index) => (
                <span key={line} className={index === 0 ? 'text-ui font-semibold text-ink' : undefined}>
                    {line}
                </span>
            ))}
        >
            <Icon size={12} strokeWidth={2} aria-hidden="true" />
            {verdictText(t, locale, trend)}
        </Toggletip>
    );
}

/** A word that tells more on demand, the `New` of a signature for instance: a chip with its tip. */
export function HintChip({ hint, children }: { hint: string; children: ReactNode }) {
    return (
        <Toggletip className={chipVariants({ tone: 'neutral' })} tip={hint}>
            {children}
        </Toggletip>
    );
}

/** A category of drops: its icon and its word, as the log and the profile name a drop. */
export function CategoryWord({ category }: { category: DropCategory }) {
    const { t } = useI18n();
    const Icon = DROP_ICONS[category];
    return (
        <span className="inline-flex items-center gap-1.5 whitespace-nowrap text-ink">
            <Icon size={14} strokeWidth={1.5} className="text-ink-3" aria-hidden="true" />
            {t.drops.categories[category]}
        </span>
    );
}

/** A share of a ranking as a bar: the first row's in the accent, the others grey. */
export function ShareBar({ share, first }: { share: number; first: boolean }) {
    return (
        <div className="h-1 min-w-16 rounded-[2px] bg-surface-3" aria-hidden="true">
            <span
                className={cn('block h-full rounded-[2px]', first ? 'bg-accent' : 'bg-chart-muted')}
                style={{ width: `${Math.max(0, Math.min(1, share)) * 100}%` }}
            />
        </div>
    );
}
