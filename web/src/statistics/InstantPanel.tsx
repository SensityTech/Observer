import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { CircleHelp } from 'lucide-react';
import type { ReactNode } from 'react';

import type { EventLink } from '../api/generated/EventLink';
import type { Timeline } from '../api/generated/Timeline';
import type { TimelineEvent } from '../api/generated/TimelineEvent';
import { dismissLink } from '../api/statistics';
import { Button, buttonVariants } from '../components/Button';
import { errorMessage } from '../i18n/errors';
import { formatTimestamp } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { eventWords, startOf } from './events';
import { changeOver, timeOf, valuesOf, type LaneKey } from './lanes';

/** A change of players is told over this long before the cursor. */
const CHANGE_SECONDS = 5 * 60;

export type EventWordings = Parameters<typeof eventWords>[2];

/**
 * Beside the timeline: every lane's value at the cursor's bucket (the newest measured one without a
 * cursor), what happened around it, and the window's likely links.
 */
export function InstantPanel({
    timeline,
    bucket,
    hovered,
    lanes,
    around,
    links,
    events,
    cores,
    words,
}: {
    timeline: Timeline;
    bucket: number;
    hovered: boolean;
    lanes: readonly LaneKey[];
    around: readonly TimelineEvent[];
    links: readonly EventLink[];
    events: readonly TimelineEvent[];
    cores: number | null;
    words: EventWordings;
}) {
    const { t, format } = useI18n();
    const s = t.statistics.instant;
    const at = (series: Parameters<typeof valuesOf>[1]) => valuesOf(timeline, series)[bucket] ?? null;
    const time = timeOf(timeline, bucket);
    const frames = timeline.frames;
    const ms = (value: number | null | undefined) => {
        if (value === null || value === undefined) {
            return s.unknown;
        }
        if (frames?.floor != null && value <= frames.floor) {
            return s.below(format.number(frames.floor));
        }
        if (frames?.top != null && value >= frames.top) {
            return s.atLeast(format.number(frames.top));
        }
        return format.milliseconds(value);
    };

    const rows: { term: string; value: ReactNode }[] = [];
    for (const lane of lanes) {
        switch (lane) {
            case 'players': {
                const players = at('players');
                const change = changeOver(timeline, 'players', bucket, CHANGE_SECONDS);
                rows.push({
                    term: s.players,
                    value: (
                        <>
                            <b className="font-semibold text-ink">
                                {players === null ? s.unknown : format.number(Math.round(players))}
                            </b>
                            {change !== null && Math.round(change) !== 0 && (
                                <span className="ml-1.5 text-small text-ink-3">
                                    {s.change(
                                        format.number(Math.round(change), { signDisplay: 'exceptZero' }),
                                    )}
                                </span>
                            )}
                        </>
                    ),
                });
                break;
            }
            case 'frames': {
                const median = frames?.p50[bucket] ?? null;
                const p99 = frames?.p99[bucket] ?? null;
                rows.push({
                    term: s.frames,
                    value:
                        median === null && p99 === null ? (
                            <>
                                <b className="font-semibold text-ink">{ms(at('mainTick'))}</b>{' '}
                                <span className="text-small text-ink-3">{s.mean}</span>
                            </>
                        ) : (
                            <>
                                <b className="font-semibold text-ink">{ms(median)}</b>{' '}
                                <span className="text-small text-ink-3">{s.median}</span> ·{' '}
                                <b
                                    className={
                                        p99 !== null && p99 >= 50
                                            ? 'font-semibold text-warning'
                                            : 'font-semibold text-ink'
                                    }
                                >
                                    {ms(p99)}
                                </b>{' '}
                                <span className="text-small text-ink-3">{s.p99}</span>
                            </>
                        ),
                });
                break;
            }
            case 'cpu': {
                const total = at('fxsCpu');
                const main = at('mainThreadCpu');
                rows.push({
                    term: s.cpu,
                    value: (
                        <>
                            <b className="font-semibold text-ink">
                                {total === null ? s.unknown : format.percent(total)}
                            </b>
                            {main !== null && (
                                <span className="ml-1.5 text-small text-ink-3">
                                    {s.main(format.percent(main))}
                                </span>
                            )}
                            {total !== null && cores !== null && cores > 0 && (
                                <span className="block text-small text-ink-3">
                                    {s.machine(format.percent(total / cores))}
                                </span>
                            )}
                        </>
                    ),
                });
                break;
            }
            case 'memory': {
                const used = at('fxsPrivate');
                rows.push({
                    term: s.memory,
                    value: (
                        <b className="font-semibold text-ink">
                            {used === null ? s.unknown : format.bytes(used)}
                        </b>
                    ),
                });
                break;
            }
            case 'ping': {
                const average = at('pingAvg');
                const p95 = at('pingP95');
                rows.push({
                    term: s.ping,
                    value: (
                        <>
                            <b className="font-semibold text-ink">
                                {average === null ? s.unknown : format.milliseconds(average)}
                            </b>
                            {p95 !== null && (
                                <span className="ml-1.5 text-small text-ink-3">
                                    {s.p95} {format.milliseconds(p95)}
                                </span>
                            )}
                        </>
                    ),
                });
                break;
            }
            case 'network': {
                const received = at('hostReceive');
                const sent = at('hostSend');
                rows.push({
                    term: s.network,
                    value:
                        received === null || sent === null
                            ? s.unknown
                            : s.inOut(format.bitRate(received), format.bitRate(sent)),
                });
                break;
            }
            case 'heap': {
                const used = at('jsHeapUsed');
                const total = at('jsHeapTotal');
                rows.push({
                    term: s.heap,
                    value:
                        used === null
                            ? s.unknown
                            : total === null
                              ? format.bytes(used)
                              : s.ofTotal(format.bytes(used), format.bytes(total)),
                });
                break;
            }
            case 'disk': {
                const latency = at('hostDiskLatency');
                rows.push({
                    term: s.disk,
                    value: latency === null ? s.unknown : format.milliseconds(latency),
                });
                break;
            }
        }
    }

    return (
        <aside
            className="card flex min-w-0 flex-1 basis-[300px] flex-col"
            aria-label={s.label(formatTimestamp(new Date(time * 1000), { seconds: false }))}
        >
            <div className="card-head justify-between">
                <h2 className="font-mono text-ui font-semibold text-ink">
                    {formatTimestamp(new Date(time * 1000), { seconds: false })}
                </h2>
                <span className="text-small text-ink-3">{hovered ? s.under : s.latest}</span>
            </div>
            <dl className="flex flex-col px-5 py-1.5 max-md:px-4" aria-live="off">
                {rows.map((row) => (
                    <div
                        key={row.term}
                        className="flex items-baseline justify-between gap-3 border-b border-line py-2.25 last:border-b-0"
                    >
                        <dt className="text-ui text-ink-3">{row.term}</dt>
                        <dd className="text-right text-ui text-ink-2">{row.value}</dd>
                    </div>
                ))}
            </dl>
            <div className="flex flex-col gap-2.5 border-t border-line px-5 py-3 max-md:px-4">
                <span className="text-eyebrow tracking-label text-ink-3 uppercase">{s.around}</span>
                {around.length === 0 ? (
                    <p className="text-small text-ink-3">{s.nothing}</p>
                ) : (
                    <ul className="flex flex-col gap-2">
                        {around.map((event) => (
                            <EventLine key={event.key} event={event} words={words} />
                        ))}
                    </ul>
                )}
            </div>
            {links.length > 0 && (
                <div className="mt-auto flex flex-col gap-2 p-3">
                    {links.map((link) => (
                        <LinkBox key={link.key} link={link} events={events} words={words} />
                    ))}
                </div>
            )}
        </aside>
    );
}

/** One event as the panel and the window's list say it: its time, its words, where to look. */
export function EventLine({ event, words }: { event: TimelineEvent; words: EventWordings }) {
    const { t } = useI18n();
    const e = t.statistics.events;
    const start = startOf(event.key);
    const signature = event.detail.kind === 'crashWave' ? event.detail.signatureId : null;
    return (
        <li className="flex items-start gap-2.5">
            <span className="w-11 shrink-0 font-mono text-small text-ink-3">{words.clock(event.at)}</span>
            <span className="min-w-0 flex-1 text-small text-ink-2 [overflow-wrap:anywhere]">
                {eventWords(t, event, words)}
            </span>
            {start !== null && (
                <Link
                    to="/changes"
                    search={{ to: start }}
                    className="text-small text-ink-2 underline-offset-2 hover:underline"
                >
                    {e.diff}
                </Link>
            )}
            {signature !== null && (
                <Link
                    to="/drops"
                    search={{ category: 'crash', signature }}
                    className="text-small text-ink-2 underline-offset-2 hover:underline"
                >
                    {e.drops}
                </Link>
            )}
        </li>
    );
}

/** A crash wave after a change, in words, with the way to look and the way to set it aside. */
function LinkBox({
    link,
    events,
    words,
}: {
    link: EventLink;
    events: readonly TimelineEvent[];
    words: EventWordings;
}) {
    const { t, format } = useI18n();
    const l = t.statistics.link;
    const queryClient = useQueryClient();
    const change = events.find((event) => event.key === link.change);
    const start = startOf(link.change);
    const mark = useMutation({
        mutationFn: (dismissed: boolean) => dismissLink({ link: link.key, dismissed }),
        onSuccess: () => queryClient.invalidateQueries({ queryKey: ['metrics', 'events'] }),
    });
    if (link.dismissed) {
        return (
            <p className="flex flex-wrap items-center gap-2 rounded-box bg-surface-2 px-3.5 py-2.5 text-small text-ink-3">
                {l.dismissed}
                <Button size="sm" tone="ghost" pending={mark.isPending} onClick={() => mark.mutate(false)}>
                    {l.undo}
                </Button>
            </p>
        );
    }
    return (
        <section className="flex flex-col gap-2 rounded-box bg-warning-soft px-3.5 py-3" aria-label={l.title}>
            <span className="flex items-center gap-2 text-small font-semibold text-warning">
                <CircleHelp size={14} strokeWidth={2} aria-hidden="true" />
                {l.title}
            </span>
            <p className="text-small text-ink-2">
                {l.sentence(
                    format.duration(link.after),
                    change === undefined ? link.change : eventWords(t, change, words),
                )}
                {link.names !== null && ` ${l.names(link.names)}`}
            </p>
            <div className="flex flex-wrap gap-2">
                {start !== null && (
                    <Link to="/changes" search={{ to: start }} className={buttonVariants({ size: 'sm' })}>
                        {l.openDiff}
                    </Link>
                )}
                <Button size="sm" tone="ghost" pending={mark.isPending} onClick={() => mark.mutate(true)}>
                    {l.notRelated}
                </Button>
            </div>
            {mark.isError && (
                <p role="alert" className="text-small text-danger-ink">
                    {l.failed} {errorMessage(t, mark.error)}
                </p>
            )}
        </section>
    );
}
