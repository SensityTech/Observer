import {
    History,
    ListFilter,
    OctagonX,
    Package,
    RotateCw,
    TimerOff,
    Zap,
    type LucideIcon,
} from 'lucide-react';
import type { ReactNode } from 'react';

import type { ChangeEvent } from '../api/generated/ChangeEvent';
import type { DropChanges } from '../api/generated/DropChanges';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { Notice } from '../components/Notice';
import { FIGURE, formatClock, formatDay } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import type { Messages } from '../i18n/en';
import { cn } from '../lib/cn';
import { formatRate, VerdictChip } from './words';

type Tone = 'neutral' | 'warning' | 'danger';

const NODE: Record<Tone, string> = {
    neutral: 'bg-surface-2 text-ink-2 shadow-[0_0_0_4px_var(--surface)]',
    warning: 'bg-surface text-warning shadow-[inset_0_0_0_28px_var(--warning-soft),0_0_0_4px_var(--surface)]',
    danger: 'bg-surface text-danger-ink shadow-[inset_0_0_0_28px_var(--danger-soft),0_0_0_4px_var(--surface)]',
};

/** How an event of the timeline looks: its node's tone and icon, and its chip. */
const look = (t: Messages, event: ChangeEvent): { tone: Tone; icon: LucideIcon; chip: string } => {
    const chips = t.drops.changes.chips;
    switch (event.kind) {
        case 'restart':
            return { tone: 'neutral', icon: RotateCw, chip: chips.restart };
        case 'build':
            return { tone: 'neutral', icon: Package, chip: chips.build };
        case 'serverCrash':
            return event.hang
                ? { tone: 'warning', icon: TimerOff, chip: chips.hang }
                : { tone: 'danger', icon: OctagonX, chip: chips.serverCrash };
        case 'incident':
            return { tone: 'warning', icon: Zap, chip: chips.incident };
    }
};

/**
 * Restarts, FXServer builds, server crashes and incidents, newest first, each with what followed:
 * worded "after", never "caused by". Until file changes is installed, nothing else is known.
 */
export function ChangesCard({
    changes,
    failed,
    stale,
    zone,
    onShowRun,
}: {
    changes: DropChanges | undefined;
    failed: boolean;
    /** Still the previous range's, while the new one is read. */
    stale: boolean;
    zone: string;
    /** Narrows the log to a run; only for a viewer who sees the log. */
    onShowRun?: (run: number) => void;
}) {
    const { t } = useI18n();
    const words = t.drops.changes;
    let body: ReactNode;
    if (changes === undefined) {
        body = failed ? (
            <div className="p-5 max-md:p-4">
                <Notice tone="danger" live={false}>
                    {words.failed}
                </Notice>
            </div>
        ) : (
            <div className="flex flex-col gap-3 p-5" aria-busy="true">
                <span className="sr-only">{words.loading}</span>
                <span className="skeleton w-3/4" />
                <span className="skeleton w-1/2" />
            </div>
        );
    } else if (changes.events.length === 0) {
        body = <p className="px-6 py-4 text-center text-ui text-ink-3">{words.empty}</p>;
    } else {
        body = (
            <ol className="flex flex-col px-5 pt-2 pb-2.5 max-md:px-4" aria-label={words.caption}>
                {changes.events
                    .toSorted((a, b) => Date.parse(b.at) - Date.parse(a.at))
                    .map((event) => (
                        <Event
                            key={`${event.kind}-${event.at}-${event.run}`}
                            event={event}
                            zone={zone}
                            onShowRun={onShowRun}
                        />
                    ))}
            </ol>
        );
    }
    return (
        <Card id="drops-changes" icon={History} title={words.title} flush>
            <div className="flex h-full flex-col">
                <div aria-busy={stale} className={cn('flex-1 transition-opacity', stale && 'opacity-50')}>
                    {body}
                </div>
                {changes !== undefined && !changes.snapshots && (
                    <p className="rounded-b-card border-t border-line bg-bg-subtle px-5 py-2.5 text-meta text-ink-3 max-md:px-4">
                        {words.snapshots}
                    </p>
                )}
            </div>
        </Card>
    );
}

function Event({
    event,
    zone,
    onShowRun,
}: {
    event: ChangeEvent;
    zone: string;
    onShowRun?: (run: number) => void;
}) {
    const { t, locale, format } = useI18n();
    const words = t.drops.changes;
    const { tone, icon: Icon, chip } = look(t, event);
    const facts = [formatDay(locale, event.at, zone), words.run(event.run)];
    let sentence: ReactNode = words.restart;
    let more: ReactNode = null;
    if (event.kind === 'build') {
        sentence = words.build(event.buildFrom, event.buildTo);
        more = (
            <>
                {event.beforeAfter.map(({ category, trend }) => (
                    <p
                        key={category}
                        className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-small text-ink-2"
                    >
                        <span>
                            {words.after(
                                t.drops.categoriesInline[category],
                                trend.before === null ? t.drops.unknown : formatRate(locale, trend.before),
                                trend.now === null ? t.drops.unknown : formatRate(locale, trend.now),
                            )}
                        </span>
                        <VerdictChip trend={trend} windows={t.drops.windows.change} />
                    </p>
                ))}
                {event.newSignatures.length > 0 && (
                    <p className="mt-1 text-small leading-[1.7] text-ink-2">
                        {words.newSignatures(
                            event.newSignatures.length,
                            event.newSignatures.map((signature, index) => (
                                <span key={signature}>
                                    {index > 0 && ' '}
                                    <code className="code-inline whitespace-normal [overflow-wrap:anywhere]">
                                        {signature}
                                    </code>
                                </span>
                            )),
                        )}
                    </p>
                )}
            </>
        );
    } else if (event.kind === 'serverCrash') {
        sentence = event.hang ? words.hang(event.playersDropped) : words.crash(event.playersDropped);
    } else if (event.kind === 'incident') {
        sentence = words.incident(
            event.drops,
            event.hitchMs === null
                ? null
                : event.hitchMs < 1000
                  ? format.milliseconds(event.hitchMs)
                  : words.seconds(format.number(event.hitchMs / 1000, FIGURE)),
        );
        facts.push(
            ...event.incidentCategories.map((each) =>
                words.incidentCategory(each.drops, t.drops.categoriesInline[each.category]),
            ),
        );
        if (event.playersOnline !== null) {
            facts.push(words.online(event.playersOnline));
        }
    }
    const linked = onShowRun !== undefined && (event.kind === 'serverCrash' || event.kind === 'incident');
    return (
        <li className="relative grid grid-cols-[40px_28px_minmax(0,1fr)] gap-x-3 py-2.75 before:absolute before:top-0 before:bottom-0 before:left-[66px] before:w-px before:bg-line-strong first:before:top-[25px] last:before:bottom-[calc(100%-25px)] only:before:hidden max-md:before:left-[66px]">
            <time
                dateTime={event.at}
                className="pt-1.25 text-right font-mono text-meta text-ink-3 tabular-nums"
            >
                {formatClock(locale, event.at, zone)}
            </time>
            <span
                className={cn('relative grid size-7 place-items-center rounded-sm', NODE[tone])}
                aria-hidden="true"
            >
                <Icon size={14} strokeWidth={1.5} />
            </span>
            <div className="min-w-0">
                <p className="text-nav leading-[1.55] text-pretty text-ink-2 [overflow-wrap:anywhere]">
                    <Chip tone={tone} icon={tone === 'neutral' ? undefined : Icon} className="mr-1.5">
                        {chip}
                    </Chip>
                    {sentence}
                </p>
                {more}
                <div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-meta text-ink-3">
                    <span>{facts.join(' · ')}</span>
                    {linked && (
                        <Button
                            size="sm"
                            tone="ghost"
                            icon={ListFilter}
                            className="-my-1 h-6.5 px-1.5"
                            onClick={() => onShowRun(event.run)}
                        >
                            {words.showInLog}
                        </Button>
                    )}
                </div>
            </div>
        </li>
    );
}
