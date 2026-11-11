import { ScanSearch, X } from 'lucide-react';
import { useState, type ReactNode } from 'react';

import { Permission } from '../api/generated/Permission';
import type { ResourceRow } from '../api/generated/ResourceRow';
import type { ResourcesRescan } from '../api/generated/ResourcesRescan';
import type { ResourceVerb } from '../api/resources';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { Find } from '../debug/parts';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useNow } from '../lib/useNow';
import { useLinkState } from '../live/context';
import { startTotal } from '../overview/StartCard';
import { RestartDialog, StopDialog } from '../resources/Dialogs';
import { Folder } from '../resources/Folder';
import { countByFilter, FILTERS, groupByFolder, type Filter } from '../resources/list';
import { useResources, type Outcome } from '../resources/live';
import { RESCAN, underWay } from '../resources/sent';
import type { Order } from '../resources/words';
import { useServerStatus } from '../server/context';

const FOLDED_KEY = 'observer.resources.folded';
/** What happened to a resource reads to the minute: the page counts again this often. */
const NOW_EVERY = 15_000;
/** What a rescan found stays on the page this long. */
const RESCAN_SHOWN = 5 * 60_000;

const readFolded = (): ReadonlySet<string> => {
    try {
        const saved: unknown = JSON.parse(localStorage.getItem(FOLDED_KEY) ?? '[]');
        return new Set(
            Array.isArray(saved) ? saved.filter((key): key is string => typeof key === 'string') : [],
        );
    } catch {
        return new Set();
    }
};

const keepFolded = (folded: ReadonlySet<string>) => {
    try {
        localStorage.setItem(FOLDED_KEY, JSON.stringify([...folded]));
    } catch {
        // Private browsing: the folds last for this page only.
    }
};

/** What stands in the list's place while the server has none to give. */
function Empty({ led, title, text }: { led: string; title: string; text: string }) {
    return (
        <section className="card dot-grid grid place-items-center px-5 py-14">
            <div className="flex max-w-empty flex-col items-center gap-1.5 text-center">
                <h2 className="flex items-center gap-2 text-body font-semibold text-ink">
                    <span className={cn('led', led)} aria-hidden="true" />
                    {title}
                </h2>
                <p className="text-small text-pretty text-ink-2">{text}</p>
            </div>
        </section>
    );
}

/** What FXServer found as it read its folders again: the new resources by name, the manifests that changed. */
function Rescanned({ rescan }: { rescan: ResourcesRescan }) {
    const { t } = useI18n();
    const words = t.resources.rescanned;
    const changed = rescan.changed ?? 0;
    if (rescan.addedCount === 0 && changed === 0) {
        return <Notice tone="success">{words.nothing}</Notice>;
    }
    const what: ReactNode[] = [];
    if (rescan.addedCount > 0) {
        what.push(words.added(rescan.addedCount));
        for (const name of rescan.added) {
            what.push(
                ', ',
                <span key={name} className="font-mono [font-variant-ligatures:none]">
                    {name}
                </span>,
            );
        }
        if (rescan.addedCount > rescan.added.length) {
            what.push(', ', words.more(rescan.addedCount - rescan.added.length));
        }
    }
    if (changed > 0) {
        what.push(what.length > 0 ? words.and : '', words.changed(changed));
    }
    return <Notice tone="success">{words.found(what)}</Notice>;
}

/** An order that did not simply go through, until it is dismissed or another is made. */
function OrderNotice({ outcome, onDismiss }: { outcome: Outcome; onDismiss: () => void }) {
    const { t } = useI18n();
    const words = t.resources;
    const text =
        outcome.kind === 'pending'
            ? t.console.pending
            : outcome.kind === 'busy'
              ? t.console.busy
              : words.orderFailed(
                    outcome.verb === 'rescan' ? words.rescanning : words.ordering[outcome.verb](outcome.name),
                    errorMessage(t, outcome.error),
                );
    return (
        <Notice tone={outcome.kind === 'pending' ? 'warning' : 'danger'}>
            <div className="flex items-start gap-3">
                <p className="flex-1">{text}</p>
                <button
                    type="button"
                    className="-m-1 rounded-sm p-1"
                    aria-label={t.controls.dismiss}
                    onClick={onDismiss}
                >
                    <X size={14} strokeWidth={1.5} aria-hidden="true" />
                </button>
            </div>
        </Notice>
    );
}

/**
 * Every resource FXServer knows in the run that is up, by folder, as `server.resources` says them,
 * with a start, a stop or a restart of each for who may. A stop or a restart that takes other
 * resources down asks first, and names them.
 */
export function ResourcesPage() {
    const { t, format } = useI18n();
    const words = t.resources;
    const me = useMe();
    const canAct = allows(me, Permission.ResourcesControl);
    const { view, sent, outcome, dismiss, order, rescan } = useResources();
    const link = useLinkState();
    const { status } = useServerStatus();
    const now = useNow(NOW_EVERY);
    const [filter, setFilter] = useState<Filter>('all');
    const [find, setFind] = useState('');
    const [folded, setFolded] = useState(readFolded);
    const [confirming, setConfirming] = useState<{ verb: 'restart' | 'stop'; name: string } | null>(null);

    const listed = view !== null && view.stage === 'listed' ? view : null;
    const rows = listed?.resources ?? [];
    const target = confirming === null ? undefined : rows.find((row) => row.name === confirming.name);
    // The resource went down or away, or took another order, while the question stood.
    if (
        confirming !== null &&
        (target === undefined || target.state !== 'started' || underWay(target, sent) !== null)
    ) {
        setConfirming(null);
    }

    const orderOf = (row: ResourceRow): Order | null => {
        const verb = underWay(row, sent);
        if (verb === null) {
            return null;
        }
        return { verb, by: sent.has(row.name) ? me.username : (row.asked?.by ?? me.username) };
    };
    const ask = (verb: ResourceVerb, row: ResourceRow) => {
        if (verb !== 'start' && row.dependants.length > 0) {
            setConfirming({ verb, name: row.name });
        } else {
            order(verb, row.name);
        }
    };
    const fold = (key: string, on: boolean) => {
        const next = new Set(folded);
        if (on) {
            next.add(key);
        } else {
            next.delete(key);
        }
        setFolded(next);
        keepFolded(next);
    };

    const counts = countByFilter(rows);
    const groups = groupByFolder(rows, filter, find);
    const start = status?.start ?? null;
    const rescanned =
        listed !== null && listed.rescan !== null && now - Date.parse(listed.rescan.at) < RESCAN_SHOWN
            ? listed.rescan
            : null;

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">{words.title}</h1>
                {canAct && listed !== null && (
                    <Button
                        icon={ScanSearch}
                        pending={sent.has(RESCAN) || listed.rescanning}
                        onClick={rescan}
                    >
                        {words.rescan}
                    </Button>
                )}
            </div>
            <p className="-mt-2 max-w-3xl text-body text-pretty text-ink-2">{words.lead}</p>
            {outcome !== null && <OrderNotice outcome={outcome} onDismiss={dismiss} />}
            {view === null &&
                (link === 'offline' || link === 'refused' ? (
                    <Notice tone="warning">{words.unavailable}</Notice>
                ) : (
                    <div className="skeleton h-40 rounded-card" aria-busy="true">
                        <span className="sr-only">{t.common.loading}</span>
                    </div>
                ))}
            {view?.stage === 'stopped' && (
                <Empty led="led-off" title={words.empty.stopped.title} text={words.empty.stopped.text} />
            )}
            {view?.stage === 'starting' && (
                <Empty
                    led="led-pending"
                    title={words.empty.starting.title}
                    text={words.empty.starting.text(
                        start?.started ?? null,
                        start === null ? null : startTotal(start),
                    )}
                />
            )}
            {view?.stage === 'unreported' && (
                <Empty
                    led="led-warning"
                    title={words.empty.unreported.title}
                    text={words.empty.unreported.text}
                />
            )}
            {listed !== null && (
                <>
                    {listed.truncated && <Notice tone="warning">{words.truncated(rows.length)}</Notice>}
                    {rescanned !== null && <Rescanned rescan={rescanned} />}
                    <div className="flex flex-wrap items-center gap-2.5">
                        <Segmented
                            label={words.filtersLabel}
                            choices={FILTERS}
                            value={filter}
                            words={words.filters}
                            counts={{
                                all: format.number(counts.all),
                                started: format.number(counts.started),
                                stopped: format.number(counts.stopped),
                                failed: format.number(counts.failed),
                                warned: format.number(counts.warned),
                            }}
                            onChange={setFilter}
                        />
                        <Find label={words.find} value={find} onChange={setFind} />
                    </div>
                    {groups.length === 0 ? (
                        <p className="card px-6 py-8 text-center text-ui text-ink-3">
                            {rows.length === 0 ? words.none : words.noMatch}
                        </p>
                    ) : (
                        <div className="flex flex-col gap-3">
                            {groups.map((group) => (
                                <Folder
                                    key={group.key}
                                    group={group}
                                    now={now}
                                    folded={folded.has(group.key)}
                                    onFold={(on) => fold(group.key, on)}
                                    orderOf={orderOf}
                                    onAsk={canAct ? ask : null}
                                />
                            ))}
                        </div>
                    )}
                </>
            )}
            {confirming?.verb === 'restart' && target !== undefined && (
                <RestartDialog
                    row={target}
                    onClose={() => setConfirming(null)}
                    onConfirm={(again) => {
                        setConfirming(null);
                        order('restart', target.name, again);
                    }}
                />
            )}
            {confirming?.verb === 'stop' && target !== undefined && (
                <StopDialog
                    row={target}
                    onClose={() => setConfirming(null)}
                    onConfirm={() => {
                        setConfirming(null);
                        order('stop', target.name);
                    }}
                />
            )}
        </div>
    );
}
