import { useMutation, useQueryClient, type UseQueryResult } from '@tanstack/react-query';
import { Pause, Play } from 'lucide-react';
import { useState, type ReactNode } from 'react';

import { ApiError } from '../api/client';
import { featuresQuery, switchFeature } from '../api/features';
import type { DebugStatus } from '../api/generated/DebugStatus';
import type { DebugWindow } from '../api/generated/DebugWindow';
import { useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

export const WINDOWS = ['1m', '5m', '15m'] as const satisfies readonly DebugWindow[];

/** What every Debug page answers with: where the counting stands, and whether rows were left out. */
type Counted = { status: DebugStatus; truncated: boolean };

/** Why the feature answers nothing: turned off, or held off by `observer.toml`. */
const offBy = (error: unknown) =>
    error instanceof ApiError && error.code === 'feature_disabled'
        ? error.params.held === 'host'
            ? 'host'
            : 'owner'
        : null;

/**
 * A Debug page's frame: its title, where the counting stands, the window and the pause, and what
 * takes the page's place while nothing is counted. The page's own content shows once the server
 * that runs was started to count.
 */
export function DebugFrame({
    title,
    lead,
    query,
    window,
    onWindow,
    paused,
    onPause,
    children,
}: {
    title: string;
    lead: string;
    query: UseQueryResult<Counted>;
    window: DebugWindow;
    onWindow: (window: DebugWindow) => void;
    paused: boolean;
    onPause: (paused: boolean) => void;
    children: ReactNode;
}) {
    const { t } = useI18n();
    const words = t.debug;
    const status = query.data?.status ?? null;
    const off = offBy(query.error);
    const counting = off === null && status?.state === 'counting';

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">{title}</h1>
                {counting && (
                    <div className="flex flex-wrap items-center gap-2">
                        <Segmented
                            label={words.window}
                            choices={WINDOWS}
                            value={window}
                            words={words.windows}
                            onChange={onWindow}
                        />
                        <Button
                            icon={paused ? Play : Pause}
                            aria-pressed={paused}
                            onClick={() => onPause(!paused)}
                        >
                            {paused ? words.resume : words.pause}
                        </Button>
                    </div>
                )}
            </div>
            <div className="-mt-2 flex max-w-3xl flex-col gap-1.5">
                <p className="text-body text-pretty text-ink-2">{lead}</p>
                {counting && status !== null && <Counting status={status} />}
            </div>
            {query.isPending && (
                <div className="skeleton h-40 rounded-card" aria-busy="true">
                    <span className="sr-only">{t.common.loading}</span>
                </div>
            )}
            {off !== null && <Off locked={off === 'host'} />}
            {query.isError && off === null && (
                <Notice tone="danger">
                    {words.loadFailed} {errorMessage(t, query.error)}
                </Notice>
            )}
            {off === null && status?.state === 'waiting' && <Waiting running={status.running} />}
            {off === null && status?.state === 'refused' && <Refused reason={status.refused} />}
            {counting && paused && <Notice>{words.paused}</Notice>}
            {counting && query.data?.truncated === true && <Notice tone="warning">{words.truncated}</Notice>}
            {counting && children}
        </div>
    );
}

/** Since when the server counts, how many resources report, and those that do not on a click. */
function Counting({ status }: { status: DebugStatus }) {
    const { t, format } = useI18n();
    const words = t.debug;
    const [open, setOpen] = useState(false);
    return (
        <div className="flex flex-col gap-1.5">
            <p className="flex flex-wrap items-center gap-x-1.5 gap-y-1 font-mono text-meta text-ink-3 max-md:flex-col max-md:items-start">
                <span>
                    {words.counting(
                        status.since === null ? '' : format.timestamp(status.since, { seconds: false }),
                        status.reporting,
                    )}
                </span>
                {status.silentCount > 0 && (
                    <>
                        <span className="max-md:hidden" aria-hidden="true">
                            ·
                        </span>
                        <button
                            type="button"
                            aria-expanded={open}
                            className="rounded-xs underline underline-offset-2 hover:text-ink-2"
                            onClick={() => setOpen(!open)}
                        >
                            {words.silent(status.silentCount)}
                        </button>
                    </>
                )}
            </p>
            {open && (
                <div className="flex flex-col gap-1.5 rounded-box border border-line bg-surface-2 px-3.5 py-2.5">
                    <p className="text-small text-ink-2">{words.silentAbout}</p>
                    <p className="font-mono text-meta text-ink-2 [overflow-wrap:anywhere]">
                        {status.silent.join(', ')}
                        {status.silentCount > status.silent.length &&
                            ` ${words.silentMore(status.silentCount - status.silent.length)}`}
                    </p>
                </div>
            )}
        </div>
    );
}

function State({ led, children }: { led: string; children: string }) {
    return (
        <h2 className="flex items-center gap-2 text-body font-semibold text-ink">
            <span className={cn('led', led)} aria-hidden="true" />
            {children}
        </h2>
    );
}

/** The switch of Debug, from a Debug page: on from here, and off again while it waits or was refused. */
const useSwitch = () => {
    const client = useQueryClient();
    return useMutation({
        mutationFn: (enabled: boolean) => switchFeature('debug', enabled),
        onSuccess: (list) => {
            client.setQueryData(featuresQuery.queryKey, list);
            void client.invalidateQueries({ queryKey: ['debug'] });
        },
    });
};

/** Debug is off: what it is, the owner's button, and what turning it on changes in the build. */
function Off({ locked }: { locked: boolean }) {
    const { t } = useI18n();
    const words = t.debug.off;
    const me = useMe();
    const turn = useSwitch();
    return (
        <div className="grid items-start gap-4 lg:grid-cols-[minmax(0,26rem)_minmax(0,1fr)]">
            <section className="card flex flex-col gap-3 p-5 max-md:p-4">
                <State led="led-off">{words.title}</State>
                <p className="text-small text-pretty text-ink-2">{words.about}</p>
                {locked ? (
                    <Notice>{words.locked}</Notice>
                ) : me.isOwner ? (
                    <Button
                        tone="ink"
                        className="self-start"
                        pending={turn.isPending}
                        onClick={() => turn.mutate(true)}
                    >
                        {words.turnOn}
                    </Button>
                ) : (
                    <p className="text-small text-ink-3">{words.ownerOnly}</p>
                )}
                {turn.isError && <Notice tone="danger">{errorMessage(t, turn.error)}</Notice>}
            </section>
            <section className="card flex flex-col gap-4 p-5 max-md:p-4">
                <h2 className="card-title">{words.changes}</h2>
                <dl className="flex flex-col gap-3.5">
                    {words.points.map((point) => (
                        <div key={point.title} className="flex flex-col gap-0.5">
                            <dt className="text-small font-semibold text-ink">{point.title}</dt>
                            <dd className="text-small text-pretty text-ink-2">{point.text}</dd>
                        </div>
                    ))}
                </dl>
            </section>
        </div>
    );
}

/** Debug is on, and the server that runs was started without it, or none runs. */
function Waiting({ running }: { running: boolean }) {
    const { t } = useI18n();
    const words = t.debug.waiting;
    return (
        <section className="card flex max-w-2xl flex-col gap-3 p-5 max-md:p-4">
            <State led="led-warning">{words.title}</State>
            <p className="text-small text-pretty text-ink-2">{running ? words.running : words.stopped}</p>
            <TurnOff />
        </section>
    );
}

/** Debug is on, and the build's scheduler was left as it was at this start. */
function Refused({ reason }: { reason: string | null }) {
    const { t } = useI18n();
    const words = t.debug.refused;
    return (
        <section className="card flex max-w-2xl flex-col gap-3 p-5 max-md:p-4">
            <State led="led-down">{words.title}</State>
            <p className="text-small text-pretty text-ink-2">{words.about}</p>
            {reason !== null && (
                <p className="rounded-box border border-line bg-surface-2 px-3 py-2 font-mono text-meta text-ink-2 [overflow-wrap:anywhere]">
                    {words.said(reason)}
                </p>
            )}
            <TurnOff />
        </section>
    );
}

function TurnOff() {
    const { t } = useI18n();
    const me = useMe();
    const turn = useSwitch();
    if (!me.isOwner) {
        return null;
    }
    return (
        <>
            <Button className="self-start" pending={turn.isPending} onClick={() => turn.mutate(false)}>
                {t.debug.waiting.turnOff}
            </Button>
            {turn.isError && <Notice tone="danger">{errorMessage(t, turn.error)}</Notice>}
        </>
    );
}
