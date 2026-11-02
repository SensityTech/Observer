import { useQuery } from '@tanstack/react-query';
import { use, useId, type ReactNode } from 'react';

import type { RunMarker } from '../api/generated/RunMarker';
import { Permission } from '../api/generated/Permission';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { useArtifacts } from '../artifacts/context';
import { allows, MeContext } from '../auth/me';
import { formatClock, formatDay, zoneDate } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { useNow } from '../lib/useNow';
import { useLiveMetrics } from '../metrics/live';
import { useSchedule } from '../schedule/context';
import { useServerStatus } from '../server/context';
import { buildFact, causeText, isRunning } from '../server/state';
import { ChangesFact } from './ChangesFact';
import { DAY } from './Summary';

/** How many of the day's runs the Recent card lists before the run under way. */
const EARLIER_RUNS = 3;

/** A line of the cards: when, what, and a note at its end. */
type Item = { key: string; at: string | number | null; text: string; note?: string };

/**
 * A server time as the cards write it: the clock in the schedule's zone, with its day when that is
 * not today there; in UTC, said so, before the zone is known.
 */
const useWhen = () => {
    const { t, locale } = useI18n();
    const { zone } = useSchedule();
    const now = useNow(60_000);
    return (instant: string | number) => {
        const shown = zone ?? 'UTC';
        const clock = formatClock(locale, instant, shown);
        const text =
            zoneDate(instant, shown) === zoneDate(now, shown)
                ? clock
                : t.schedule.dayTime(formatDay(locale, instant, shown, { weekday: false }), clock);
        return zone === null ? t.overview.utcClock(text) : text;
    };
};

function AgendaCard({
    title,
    items,
    empty,
    children,
}: {
    title: string;
    items: Item[];
    empty: string;
    children?: ReactNode;
}) {
    const when = useWhen();
    const titleId = useId();
    return (
        <section className="card flex flex-col" aria-labelledby={titleId}>
            <div className="card-head">
                <h2 id={titleId} className="card-title">
                    {title}
                </h2>
            </div>
            <div className="card-body flex flex-1 flex-col gap-3 max-md:p-4">
                {items.length === 0 ? (
                    <p className="text-ui text-ink-3">{empty}</p>
                ) : (
                    <ul className="flex flex-col gap-2.5">
                        {items.map(({ key, at, text, note }) => (
                            <li key={key} className="flex items-baseline gap-3 text-ui">
                                <span className="min-w-11.5 shrink-0 font-mono text-small whitespace-nowrap text-ink-3">
                                    {at === null ? '' : when(at)}
                                </span>
                                <span className="min-w-0 flex-1 text-ink-2">{text}</span>
                                {note !== undefined && (
                                    <span className="shrink-0 text-small whitespace-nowrap text-ink-3">
                                        {note}
                                    </span>
                                )}
                            </li>
                        ))}
                    </ul>
                )}
                {children}
            </div>
        </section>
    );
}

/** What is due next: the next restart, a restart after a failure, a build change, a probation's end. */
export function ComingUp() {
    const { t, format } = useI18n();
    const words = t.overview.comingUp;
    const { status } = useServerStatus();
    const { schedule } = useSchedule();
    const { status: builds } = useArtifacts();
    const now = useNow();
    const when = useWhen();
    const running = status !== null && isRunning(status.state);
    const left = (instant: string) =>
        t.status.inTime(format.duration(Math.max(0, Date.parse(instant) - now) / 1000));

    const items: Item[] = [];
    if (status?.restartAt != null) {
        items.push({
            key: 'recovery',
            at: status.restartAt,
            text: words.recovery,
            note: left(status.restartAt),
        });
    }
    const next = schedule?.next ?? null;
    if (running && next !== null) {
        let text = words.restart;
        if (next.source === 'oneOff') {
            text = words.oneOff;
        } else if (next.delayedFrom !== null) {
            text = words.delayed(when(next.delayedFrom));
        }
        items.push({ key: 'restart', at: next.at, text, note: left(next.at) });
    }
    if (builds !== null && builds.running !== null) {
        if (builds.selected !== null && builds.selected !== builds.running) {
            items.push({ key: 'build', at: null, text: words.nextStart(String(builds.selected)) });
        } else if (builds.pending !== null) {
            items.push({
                key: 'build',
                at: running ? (next?.at ?? null) : null,
                text: words.pending(String(builds.pending.build)),
            });
        }
        if (
            builds.probation !== null &&
            builds.probation.build === builds.running &&
            builds.probation.passesAt !== null
        ) {
            items.push({
                key: 'probation',
                at: builds.probation.passesAt,
                text: words.probation(String(builds.probation.build)),
            });
        }
    }

    return <AgendaCard title={words.title} items={items} empty={words.nothing} />;
}

/**
 * What happened lately: why the server is in its state (unless the notice above the tiles already
 * says it), the start of the run under way and what became of its build, the day's earlier runs,
 * and what the start changed.
 */
export function Recent({ causeShown }: { causeShown: boolean }) {
    const { t, format } = useI18n();
    const words = t.overview.recent;
    const me = use(MeContext);
    const { status } = useServerStatus();
    const { latest } = useLiveMetrics();
    const { status: builds } = useArtifacts();
    const day = useQuery(DAY);
    const when = useWhen();

    const items: Item[] = [];
    const cause = status === null || causeShown ? null : causeText(t, status);
    if (status !== null && cause !== null) {
        items.push({ key: 'cause', at: status.since, text: cause });
    }
    const started = running(status) ? status.startedAt : null;
    if (started !== null) {
        const resources = latest?.values.resources ?? null;
        items.push({
            key: 'started',
            at: started,
            text: words.started(status?.build == null ? null : String(status.build)),
            note: resources === null ? undefined : words.resources(resources),
        });
    }
    // A probation under way and a build waiting for a start are what Coming up lists.
    const upcoming =
        builds !== null &&
        (builds.probation?.build === builds.running ||
            (builds.selected !== null && builds.selected !== builds.running) ||
            builds.pending !== null);
    const fact = status === null || upcoming ? null : buildFact(t, status, builds, when);
    if (fact !== null) {
        items.push({ key: 'build', at: null, text: fact });
    }
    // The run under way is told above: its own start marker would say it again.
    const since = started === null ? Infinity : Date.parse(started) / 1000 - 30;
    const earlier = (day.data?.markers ?? [])
        .filter((marker) => marker.at < since)
        .toSorted((a, b) => b.at - a.at)
        .slice(0, EARLIER_RUNS);
    for (const marker of earlier) {
        items.push(markerItem(t, marker, format));
    }

    return (
        <AgendaCard title={words.title} items={items} empty={words.nothing}>
            {me !== null && allows(me, Permission.ChangesView) && <ChangesFact />}
        </AgendaCard>
    );
}

const running = (status: ServerStatus | null): status is ServerStatus & { startedAt: string } =>
    status !== null && isRunning(status.state) && status.startedAt !== null;

function markerItem(
    t: ReturnType<typeof useI18n>['t'],
    marker: RunMarker,
    format: ReturnType<typeof useI18n>['format'],
): Item {
    const words = t.overview.recent;
    const at = marker.at * 1000;
    const text = { start: words.start, restart: words.restart, crash: words.crash }[marker.kind];
    return {
        key: `${marker.kind}-${marker.at}`,
        at,
        text,
        note:
            marker.backOnlineAt === null
                ? undefined
                : words.backOnline(format.duration(marker.backOnlineAt - marker.at)),
    };
}
