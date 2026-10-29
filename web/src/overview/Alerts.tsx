import { Link } from '@tanstack/react-router';
import {
    ArrowRight,
    OctagonX,
    ShieldAlert,
    SquareTerminal,
    TimerOff,
    TriangleAlert,
    type LucideIcon,
} from 'lucide-react';
import type { ReactNode } from 'react';

import type { Alert } from '../api/generated/Alert';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { buttonVariants } from '../components/Button';
import { Chip } from '../components/Chip';
import { useAlerts } from '../diagnostics/alerts';
import { Code, exceptionWords, exitWords, threadWord } from '../diagnostics/words';
import type { Messages } from '../i18n/en';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

type Tone = 'warning' | 'danger';

const NODE: Record<Tone, string> = {
    warning: 'text-warning shadow-[inset_0_0_0_28px_var(--warning-soft),0_0_0_4px_var(--surface)]',
    danger: 'text-danger-ink shadow-[inset_0_0_0_28px_var(--danger-soft),0_0_0_4px_var(--surface)]',
};

/** The alerts of the last 24 hours, newest first, while there is any: shells the tree started, crashes and hangs. */
export function Alerts() {
    const { t } = useI18n();
    const alerts = useAlerts();
    if (alerts.length === 0) {
        return null;
    }
    return (
        <section className="card" aria-labelledby="overview-alerts">
            <div className="card-head wrap-push">
                <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                    <ShieldAlert size={14} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <h2 id="overview-alerts" className="text-card text-ink">
                    {t.alerts.title}
                </h2>
                <span className="text-meta text-ink-3" data-push>
                    {t.alerts.meta}
                </span>
            </div>
            <ol className="flex flex-col px-5 pt-2 pb-2.5 max-md:px-4" aria-label={t.alerts.caption}>
                {alerts.map((alert) => (
                    <AlertEvent key={alert.key} alert={alert} />
                ))}
            </ol>
            <AlertsFooter />
        </section>
    );
}

/** The way to the Diagnostics page, or why the account cannot open it. */
function AlertsFooter() {
    const { t } = useI18n();
    const me = useMe();
    return (
        <div className="flex flex-wrap items-center gap-2 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3">
            {allows(me, Permission.DiagnosticsView) ? (
                <Link
                    to="/diagnostics"
                    className={cn(buttonVariants({ tone: 'ghost', size: 'sm' }), 'ml-auto max-md:w-full')}
                >
                    {t.alerts.open}
                    <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                </Link>
            ) : (
                // Disabled but still reachable, so that the keyboard finds why.
                <button
                    type="button"
                    aria-disabled="true"
                    title={t.alerts.needsPermission}
                    className={cn(
                        buttonVariants({ tone: 'ghost', size: 'sm' }),
                        'ml-auto cursor-not-allowed opacity-45 max-md:w-full',
                    )}
                >
                    {t.alerts.open}
                    <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                    <span className="sr-only">{t.alerts.needsPermission}</span>
                </button>
            )}
        </div>
    );
}

function AlertEvent({ alert }: { alert: Alert }) {
    const { t, format } = useI18n();
    const { tone, icon: Icon, chip, sentence, facts } = describe(t, alert);
    const time = format.dateTime(new Date(alert.lastAt), { dateStyle: undefined });
    return (
        <li className="relative grid grid-cols-[40px_28px_minmax(0,1fr)] gap-x-3 py-2.75 before:absolute before:top-0 before:bottom-0 before:left-[66px] before:w-px before:bg-line-strong first:before:top-[25px] last:before:bottom-[calc(100%-25px)] only:before:hidden">
            <time
                dateTime={alert.lastAt}
                className="pt-1.25 text-right font-mono text-meta text-ink-3 tabular-nums"
            >
                {time}
            </time>
            <span
                className={cn('relative grid size-7 place-items-center rounded-[8px] bg-surface', NODE[tone])}
                aria-hidden="true"
            >
                <Icon size={14} strokeWidth={1.5} />
            </span>
            <div className="min-w-0">
                <p className="text-nav leading-[1.55] text-pretty text-ink-2 [overflow-wrap:anywhere]">
                    <Chip tone={tone} icon={tone === 'warning' ? TriangleAlert : OctagonX} className="mr-1.5">
                        {chip}
                    </Chip>
                    {sentence}
                </p>
                <p className="mt-0.5 text-meta text-ink-3">{[time, ...facts].join(' · ')}</p>
            </div>
        </li>
    );
}

/** An alert in words: its chip, its sentence and the facts after its time. */
const describe = (
    t: Messages,
    alert: Alert,
): { tone: Tone; icon: LucideIcon; chip: string; sentence: ReactNode; facts: string[] } => {
    const crash = alert.crash;
    if (alert.kind === 'shell' || crash === null) {
        return {
            tone: 'warning',
            icon: SquareTerminal,
            chip: t.alerts.securityAlert,
            sentence: (
                <>
                    {t.alerts.shellStarted} <Code>{alert.name ?? '?'}</Code>
                </>
            ),
            facts: [
                alert.count > 1 ? t.alerts.times(alert.count) : null,
                alert.parent === null ? null : t.alerts.parent(alert.parent),
                t.alerts.leftRunning,
            ].filter((fact) => fact !== null),
        };
    }
    const thread = threadWord(crash.thread);
    if (alert.kind === 'hang') {
        let sentence: ReactNode = t.alerts.stuck(thread);
        if (crash.busy) {
            sentence = t.alerts.busy(thread);
        } else if (crash.waitingIn !== null) {
            sentence = (
                <>
                    {t.alerts.waitingIn(thread)} <Code>{crash.waitingIn}</Code>
                </>
            );
        } else if (crash.module !== null) {
            sentence = (
                <>
                    {t.alerts.stuckIn(thread)} <Code>{crash.module}</Code>
                </>
            );
        }
        return {
            tone: 'warning',
            icon: TimerOff,
            chip: t.alerts.hang,
            sentence,
            facts: crash.snapshot ? [t.alerts.snapshot] : [],
        };
    }
    let sentence: ReactNode = t.alerts.crashed;
    if (crash.exception !== null) {
        const words = exceptionWords(t, crash.exception.code, crash.exception.name);
        sentence =
            crash.module === null ? (
                words
            ) : (
                <>
                    {words} {t.alerts.crashedIn} <Code>{crash.module}</Code>
                </>
            );
    } else if (crash.exit !== null) {
        sentence = t.alerts.exitedWith(exitWords(t, crash.exit));
    }
    return {
        tone: 'danger',
        icon: OctagonX,
        chip: t.alerts.crash,
        sentence,
        facts: [
            crash.groupCount > 1 ? t.alerts.inDays(crash.groupCount) : null,
            crash.thread === null ? null : t.alerts.thread(crash.thread),
        ].filter((fact) => fact !== null),
    };
};
