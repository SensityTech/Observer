import { useMutation } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Hourglass } from 'lucide-react';
import { useId } from 'react';

import type { ServerStatus } from '../api/generated/ServerStatus';
import type { StartProgress } from '../api/generated/StartProgress';
import { Permission } from '../api/generated/Permission';
import { extendStart } from '../api/server';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Notice } from '../components/Notice';
import { ProgressBar } from '../components/Progress';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { useNow } from '../lib/useNow';
import { anchorOf } from '../settings/catalog';

/** A silence shorter than this is a resource starting, not yet a start to worry about. */
const QUIET_SECONDS = 30;

/** How many resources the start is counted against: those the cfg starts, or else those found. */
export const startTotal = (start: StartProgress) =>
    start.planned !== null && start.planned > 0 ? start.planned : start.found;

/**
 * A start as FXServer tells it: how many resources are up, the command of the cfg it is at, the
 * last resource started and since when it says nothing, with more time to give before Observer
 * takes the silence for a hang.
 */
export function StartCard({ status, start }: { status: ServerStatus; start: StartProgress }) {
    const { t, format } = useI18n();
    const words = t.overview.start;
    const me = useMe();
    const now = useNow();
    const titleId = useId();
    const slowId = useId();
    const extend = useMutation({ mutationFn: extendStart });

    const total = startTotal(start);
    const seconds = (from: string) => Math.max(0, now - Date.parse(from)) / 1000;
    const silence = seconds(start.last?.at ?? status.startedAt ?? status.since);
    const left = start.stuckAt === null ? null : (Date.parse(start.stuckAt) - now) / 1000;
    const slowest = start.slowest[0]?.ms ?? 0;
    const took = (ms: number) =>
        ms < 60_000
            ? words.seconds(format.number(ms / 1000, { minimumFractionDigits: 1, maximumFractionDigits: 1 }))
            : format.elapsed(ms / 1000);

    let scope: string | null = null;
    if (start.planned !== null && start.planned > 0) {
        scope = words.ofPlanned(start.planned);
    } else if (start.found !== null) {
        scope = words.ofFound(start.found);
    }

    return (
        <section className="card" aria-labelledby={titleId}>
            <div className="card-head">
                <h2 id={titleId} className="card-title">
                    {words.title(start.started)}
                </h2>
                {scope !== null && <span className="text-small text-ink-3">{scope}</span>}
            </div>
            <div className="card-body flex flex-col gap-5 max-md:p-4">
                <ProgressBar
                    label={words.progress}
                    valueText={
                        total === null ? words.title(start.started) : words.progressText(start.started, total)
                    }
                    share={total === null ? null : Math.min(1, start.started / total)}
                />
                <dl className="grid grid-cols-1 gap-x-8 gap-y-5 md:grid-cols-2">
                    <div className="flex min-w-0 flex-col gap-1">
                        <dt className="eyebrow">{words.now}</dt>
                        {start.at === null ? (
                            <dd className="text-small text-ink-3">
                                {start.planned === null ? words.unread : words.over}
                            </dd>
                        ) : (
                            <>
                                <dd className="truncate font-mono text-[15px] font-medium text-ink [font-variant-ligatures:none]">
                                    {start.at.target}
                                </dd>
                                <dd className="text-small text-ink-2 tabular-nums">
                                    {words.nowAt(
                                        start.at.file,
                                        start.at.line,
                                        format.elapsed(seconds(start.at.since)),
                                    )}
                                    {start.at.of > 1 && ` · ${words.nowOf(start.at.done, start.at.of)}`}
                                </dd>
                                <dd className="text-small text-ink-3">
                                    {start.at.resource === null
                                        ? words.quiet
                                        : words.nowScripts(start.at.resource)}
                                </dd>
                            </>
                        )}
                    </div>
                    <div className="flex min-w-0 flex-col gap-1">
                        <dt className="eyebrow">{words.last}</dt>
                        {start.last === null ? (
                            <dd className="text-small text-ink-3">{words.none}</dd>
                        ) : (
                            <>
                                <dd className="truncate font-mono text-[15px] font-medium text-ink [font-variant-ligatures:none]">
                                    {start.last.name}
                                </dd>
                                <dd className="text-small text-ink-2 tabular-nums">
                                    {words.lastAgo(
                                        format.elapsed(seconds(start.last.at)),
                                        start.last.warnings,
                                    )}
                                </dd>
                            </>
                        )}
                        {start.failed > 0 && (
                            <dd className="flex flex-wrap items-center gap-x-2 text-small text-warning">
                                {words.failed(start.failed)}
                                <Link
                                    to="/console"
                                    className="inline-flex items-center gap-1 font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                                >
                                    {words.openConsole}
                                    <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                                </Link>
                            </dd>
                        )}
                    </div>
                </dl>
                {left !== null && silence >= QUIET_SECONDS && (
                    <div className="flex flex-col gap-3 rounded-md border border-line bg-surface-2 p-4">
                        <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
                            <p className="min-w-0 flex-[1_1_22rem] text-small text-ink-2 tabular-nums">
                                {left > 0
                                    ? words.silent(format.elapsed(silence), format.elapsed(left))
                                    : words.overdue(format.elapsed(silence))}
                            </p>
                            <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
                                {allows(me, Permission.ServerControl) && (
                                    <Button
                                        size="sm"
                                        icon={Hourglass}
                                        pending={extend.isPending}
                                        onClick={() => extend.mutate()}
                                    >
                                        {words.extend(start.extensionMinutes)}
                                    </Button>
                                )}
                                {me.isOwner && (
                                    <Link
                                        to="/settings/$category"
                                        params={{ category: 'launch' }}
                                        hash={anchorOf('hangDetection')}
                                        className="inline-flex items-center gap-1.5 text-small font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                                    >
                                        {words.hangDetection}
                                        <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                                    </Link>
                                )}
                            </div>
                        </div>
                        <ProgressBar
                            tone="warning"
                            label={words.silence}
                            valueText={format.elapsed(silence)}
                            share={left > 0 ? silence / (silence + left) : 1}
                        />
                        {extend.isError && (
                            <Notice tone="danger">{words.extendFailed(errorMessage(t, extend.error))}</Notice>
                        )}
                    </div>
                )}
                {start.slowest.length > 0 && (
                    <div className="flex flex-col gap-2.5">
                        <div className="flex flex-wrap items-baseline gap-x-2.5">
                            <h3 id={slowId} className="eyebrow">
                                {words.slowest}
                            </h3>
                            <span className="text-small text-ink-3">{words.slowestHint}</span>
                        </div>
                        <ol aria-labelledby={slowId} className="flex flex-col gap-1.5">
                            {start.slowest.map((slow) => (
                                <li
                                    key={slow.name}
                                    className="grid grid-cols-[minmax(0,14rem)_minmax(0,1fr)_4.5rem] items-center gap-3 text-small max-md:grid-cols-[minmax(0,1fr)_4.5rem]"
                                >
                                    <span className="truncate font-mono text-ink-2 [font-variant-ligatures:none]">
                                        {slow.name}
                                    </span>
                                    <span
                                        className="h-1.5 overflow-hidden rounded-pill bg-surface-2 max-md:hidden"
                                        aria-hidden="true"
                                    >
                                        <span
                                            className="block h-full rounded-pill bg-ink-3"
                                            style={{
                                                width: `${slowest === 0 ? 0 : (slow.ms / slowest) * 100}%`,
                                            }}
                                        />
                                    </span>
                                    <span className="text-right text-ink-2 tabular-nums">
                                        {took(slow.ms)}
                                    </span>
                                </li>
                            ))}
                        </ol>
                    </div>
                )}
            </div>
        </section>
    );
}
