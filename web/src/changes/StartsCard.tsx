import { ChevronDown, History, OctagonX } from 'lucide-react';

import type { SnapshotSummary } from '../api/generated/SnapshotSummary';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { CAUSE_ICON, countsWords, startWords } from './words';

const CRASHED_NODE = 'text-danger-ink shadow-[inset_0_0_0_28px_var(--danger-soft),0_0_0_4px_var(--surface)]';
const NODE = 'bg-surface-2 text-ink-2 shadow-[0_0_0_4px_var(--surface)]';

/** Every start kept, newest first: what it changed, and a way to compare it with the one before. */
export function StartsCard({
    starts,
    current,
    more,
    loadingMore,
    onLoadMore,
    onPick,
}: {
    starts: SnapshotSummary[];
    /** The start compared as To. */
    current: number | null;
    more: boolean;
    loadingMore: boolean;
    onLoadMore: () => void;
    onPick: (start: SnapshotSummary) => void;
}) {
    const { t, format } = useI18n();
    const words = t.changes.starts;
    return (
        <Card id="changes-starts" icon={History} title={words.title} meta={words.meta} flush>
            <ol className="flex flex-col px-5 pt-2 pb-2.5 max-md:px-4" aria-label={words.caption}>
                {starts.map((start) => {
                    const Icon = start.crashed
                        ? OctagonX
                        : start.cause === null
                          ? History
                          : CAUSE_ICON[start.cause];
                    const time = format.timestamp(start.takenAt, { seconds: false });
                    const facts = [
                        time,
                        startWords(t, start),
                        start.build === null ? null : t.changes.build(start.build),
                        start.state === 'complete' ? null : t.changes.states[start.state],
                    ].filter((fact) => fact !== null);
                    const picked = start.id === current;
                    return (
                        <li
                            key={start.id}
                            className="relative grid grid-cols-[40px_28px_minmax(0,1fr)] gap-x-3 py-1.5 before:absolute before:top-0 before:bottom-0 before:left-[66px] before:w-px before:bg-line-strong first:before:top-[25px] last:before:bottom-[calc(100%-25px)] only:before:hidden"
                        >
                            <time
                                dateTime={start.takenAt}
                                className="pt-2.5 text-right font-mono text-meta text-ink-3 tabular-nums"
                            >
                                {format.dateTime(new Date(start.takenAt), { dateStyle: undefined })}
                            </time>
                            <span
                                className={cn(
                                    'relative mt-1.25 grid size-7 place-items-center rounded-[8px] bg-surface',
                                    start.crashed ? CRASHED_NODE : NODE,
                                )}
                                aria-hidden="true"
                            >
                                <Icon size={14} strokeWidth={1.5} />
                            </span>
                            <button
                                type="button"
                                aria-current={picked || undefined}
                                onClick={() => onPick(start)}
                                className={cn(
                                    'group min-w-0 rounded-sm px-2 py-1.25 text-left hover:bg-hover',
                                    picked && 'bg-accent-soft',
                                )}
                            >
                                <span className="sr-only">{words.compare(time)}</span>
                                <span className="flex flex-wrap items-center gap-1.5 text-nav leading-[1.55] text-ink-2 [overflow-wrap:anywhere]">
                                    {start.crashed && (
                                        <Chip tone="danger" icon={OctagonX}>
                                            {words.crashed}
                                        </Chip>
                                    )}
                                    {picked && <Chip>{words.compared}</Chip>}
                                    <span className="text-ink">{countsWords(t, start)}</span>
                                </span>
                                <span
                                    className={cn(
                                        'mt-0.5 block text-meta group-hover:text-ink-2',
                                        picked ? 'text-ink-2' : 'text-ink-3',
                                    )}
                                >
                                    {facts.join(' · ')}
                                </span>
                            </button>
                        </li>
                    );
                })}
            </ol>
            {more && (
                <div className="flex justify-center rounded-b-card border-t border-line bg-bg-subtle px-5 py-2">
                    <Button
                        size="sm"
                        tone="ghost"
                        icon={ChevronDown}
                        pending={loadingMore}
                        onClick={onLoadMore}
                    >
                        {words.loadMore}
                    </Button>
                </div>
            )}
        </Card>
    );
}
