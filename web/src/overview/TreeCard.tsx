import { useQuery } from '@tanstack/react-query';
import { Network } from 'lucide-react';
import { useId, useMemo } from 'react';

import type { Series } from '../api/generated/Series';
import { growthQuery, seriesQuery } from '../api/metrics';
import { useI18n } from '../i18n/I18nProvider';
import { useLiveMetrics } from '../metrics/live';
import { mergeLive } from '../metrics/merge';
import { Facts, total, type Fact } from './Tiles';

const STARTED = ['treeStarted'] as const satisfies readonly Series[];
/** A slot's counts are over five seconds. */
const SLOT_SECONDS = 5;

/** FXServer's process tree as the latest sample saw it: children, totals, faults, threads. */
export function TreeCard() {
    const { t, format } = useI18n();
    const titleId = useId();
    const { recent, latest } = useLiveMetrics();
    const growth = useQuery(growthQuery);
    const hour = useQuery(seriesQuery('1h', STARTED));
    const merged = useMemo(() => (hour.data ? mergeLive(hour.data, recent) : null), [hour.data, recent]);
    const value = (series: Series) => latest?.values[series] ?? null;
    const unknown = t.tiles.unknown;

    const children = value('children');
    const childrenCpu = value('childrenCpu');
    const started = total(merged?.series[0]?.values ?? []);
    const tree = value('treeCpu');
    const peak = value('treePrivatePeak');
    const faults = value('fxsFaults');
    const hard = value('fxsHardFaults');
    const threads = value('fxsThreads');
    const handles = value('fxsHandles');
    const handlesGrowth = growth.data?.handles ?? null;

    const facts: Fact[] = [
        {
            term: t.tree.children,
            detail:
                children === null
                    ? unknown
                    : t.tiles.cpu.childrenValue(
                          children,
                          childrenCpu === null ? unknown : format.percent(childrenCpu),
                      ),
        },
    ];
    // No count of processes started on Linux: the row is left out rather than always n/a.
    if (started !== null) {
        facts.push({ term: t.tree.startedLastHour, detail: format.number(started) });
    }
    facts.push({
        term: t.tree.wholeTree,
        detail:
            tree === null ? (
                unknown
            ) : (
                <>
                    {t.tree.ofOneCore(format.percent(tree))}{' '}
                    <span className="text-ink-3">{t.tree.endedIncluded}</span>
                </>
            ),
    });
    if (peak !== null) {
        facts.push({ term: t.tree.memoryPeak, detail: format.bytes(peak) });
    }
    facts.push(
        {
            term: t.tree.faults,
            detail:
                faults === null
                    ? unknown
                    : t.tree.faultsValue(
                          format.number(Math.round(faults / SLOT_SECONDS)),
                          hard === null ? null : format.number(Math.round(hard / SLOT_SECONDS)),
                      ),
        },
        {
            term: t.tiles.cpu.threadsHandles,
            detail: t.tree.threadsHandles(
                threads === null ? unknown : format.number(threads),
                handles === null ? unknown : format.number(handles),
                handlesGrowth === null
                    ? null
                    : `${handlesGrowth.perHour > 0 ? '+' : ''}${format.number(Math.round(handlesGrowth.perHour))}`,
            ),
        },
    );

    return (
        <section className="card" aria-labelledby={titleId}>
            <div className="card-head">
                <span
                    className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2"
                    aria-hidden="true"
                >
                    <Network size={14} strokeWidth={1.75} />
                </span>
                <h2 id={titleId} className="card-title">
                    {t.tree.title}
                </h2>
                <span className="ml-auto text-meta text-ink-3">{t.tree.now}</span>
            </div>
            <div className="card-body max-md:p-4">
                <Facts facts={facts} />
            </div>
        </section>
    );
}
