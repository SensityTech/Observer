import { Timer, TriangleAlert } from 'lucide-react';
import { useId, type ReactNode } from 'react';

import type { DropUptime } from '../api/generated/DropUptime';
import type { UptimeBin } from '../api/generated/UptimeBin';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { Notice } from '../components/Notice';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { formatRate, keptSince } from './words';

/**
 * Unexpected drops per 100 player-hours by hours since FXServer started, as grey bars; a bin the
 * test finds significantly higher takes the warning fill and says so in a word.
 */
export function UptimeCard({
    uptime,
    failed,
    stale,
    rangeFrom,
    zone,
}: {
    uptime: DropUptime | undefined;
    failed: boolean;
    /** Still the previous range's, while the new one is read. */
    stale: boolean;
    /** The range's start, to tell when only the sessions kept since a later day are binned. */
    rangeFrom: string | undefined;
    zone: string;
}) {
    const { t, locale } = useI18n();
    const words = t.drops.uptime;
    const since = uptime === undefined ? null : keptSince(locale, uptime.keptFrom, rangeFrom, zone);
    const summaryId = useId();
    let body: ReactNode;
    if (uptime === undefined) {
        body = failed ? (
            <Notice tone="danger" live={false}>
                {words.failed}
            </Notice>
        ) : (
            <div className="skeleton h-40 w-full" aria-busy="true" aria-label={words.loading} />
        );
    } else if (uptime.bins.every((bin) => bin.rate === null)) {
        body = (
            <div className="grid h-40 place-items-center rounded-card border border-dashed border-line-strong bg-bg-subtle px-6 text-center text-ui text-ink-3">
                {words.empty}
            </div>
        );
    } else {
        const most = Math.max(...uptime.bins.map((bin) => bin.rate ?? 0));
        const higher = uptime.bins.filter((bin) => bin.verdict === 'higher');
        const verdict = (bin: UptimeBin) =>
            bin.verdict === 'higher'
                ? words.higher
                : bin.verdict === 'notEnoughDrops'
                  ? words.notEnough
                  : null;
        const summary = words.summary(
            uptime.bins.map((bin) =>
                words.summaryBin(
                    words.bin(bin.fromHours, bin.toHours),
                    bin.rate === null ? t.drops.unknown : formatRate(locale, bin.rate),
                    verdict(bin),
                ),
            ),
        );
        body = (
            <figure className="flex flex-col" aria-labelledby={summaryId}>
                <div className="grid h-40 grid-cols-5 items-end gap-2" aria-hidden="true">
                    {uptime.bins.map((bin) => {
                        const high = bin.verdict === 'higher';
                        return (
                            <div
                                key={bin.fromHours}
                                className="flex h-full min-w-0 flex-col justify-end gap-1"
                            >
                                <span
                                    className={cn(
                                        'text-center text-small font-semibold',
                                        high ? 'text-warning' : 'text-ink',
                                    )}
                                >
                                    {bin.rate === null ? t.drops.unknown : formatRate(locale, bin.rate)}
                                </span>
                                <span
                                    className={cn(
                                        'block w-full rounded-t-xs',
                                        high ? 'bg-warning-fill' : 'bg-chart-muted',
                                    )}
                                    style={{
                                        height:
                                            bin.rate === null || most <= 0 ? 0 : `${(bin.rate / most) * 75}%`,
                                    }}
                                />
                            </div>
                        );
                    })}
                </div>
                <div className="grid grid-cols-5 gap-2 border-t border-line-strong pt-1.5" aria-hidden="true">
                    {uptime.bins.map((bin) => (
                        <span
                            key={bin.fromHours}
                            className={cn(
                                'min-w-0 text-center font-mono text-meta',
                                bin.verdict === 'higher' ? 'text-warning' : 'text-ink-3',
                            )}
                        >
                            {words.bin(bin.fromHours, bin.toHours)}
                        </span>
                    ))}
                </div>
                {higher.length > 0 && (
                    <p
                        className="mt-2.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-small text-ink-2"
                        aria-hidden="true"
                    >
                        <Chip tone="warning" icon={TriangleAlert}>
                            {words.higher}
                        </Chip>
                        {words.higherBins(higher.map((bin) => words.bin(bin.fromHours, bin.toHours)))}
                    </p>
                )}
                <figcaption id={summaryId} className="mt-4 border-t border-line pt-3 text-ui text-ink-3">
                    {summary}
                </figcaption>
            </figure>
        );
    }
    return (
        <Card id="drops-uptime" icon={Timer} title={words.title} meta={words.meta}>
            <div className="flex flex-col gap-3">
                <div aria-busy={stale} className={cn('transition-opacity', stale && 'opacity-50')}>
                    {body}
                </div>
                {since !== null && <p className="text-small text-ink-3">{words.since(since)}</p>}
                <p className="text-small text-ink-3">{words.note}</p>
            </div>
        </Card>
    );
}
