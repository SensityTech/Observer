import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowLeft, ArrowRight, Check, LoaderCircle, OctagonX, SquareTerminal } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import type { LaunchSettings } from '../api/generated/LaunchSettings';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { launchQuery } from '../api/launch';
import { Button, buttonVariants } from '../components/Button';
import { Steps } from '../components/Steps';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useServerStatus } from '../server/context';
import { HangAdvice } from '../server/HangAdvice';
import { STATE_INK, STATE_LED, causeText } from '../server/state';
import { LAUNCH_STEPS, checklist } from './state';

/**
 * The launch the setup asked for, on a page of its own, followed on `server.status` from the
 * status before it: each step ticks as it passes, and the page ends on the server online, or on
 * what stopped it. Once it is ready or failed it stays as it ended: a restart after a crash is
 * another launch.
 */
export function FirstLaunch({ before, onBack }: { before: ServerStatus | null; onBack: () => void }) {
    const { t } = useI18n();
    const words = t.serverSetup.launch;
    const { status: live } = useServerStatus();
    const launch = useQuery(launchQuery);
    const title = useRef<HTMLHeadingElement>(null);
    // The settings are kept with the end too: a delay raised since is not the one that ran out.
    const [ended, setEnded] = useState<{ status: ServerStatus; settings: LaunchSettings | null } | null>(
        null,
    );
    const status = ended?.status ?? live;
    const settings = ended === null ? (launch.data?.settings ?? null) : ended.settings;
    const list = status === null ? null : checklist(before, status);
    if (ended === null && live !== null && list !== null && !list.waiting) {
        setEnded({ status: live, settings });
    }
    const ready = list?.passed === LAUNCH_STEPS.length;
    const failed = list?.failed != null;
    const current =
        list === null
            ? -1
            : list.failed !== null
              ? LAUNCH_STEPS.indexOf(list.failed)
              : list.waiting
                ? list.passed
                : -1;
    const cause = status === null ? null : ready ? words.ready : causeText(t, status);
    const name = status?.name ?? null;

    // The keyboard comes with the page: the button that asked for the launch is gone.
    useEffect(() => title.current?.focus(), []);

    return (
        <div className="flex flex-1 justify-center px-8 pt-14 pb-16 max-md:px-4 max-md:pt-8 max-md:pb-10">
            <div className="flex w-full max-w-220 flex-col gap-7">
                <div className="flex flex-col items-center gap-4 text-center">
                    <span
                        className={cn(
                            'grid size-16 place-items-center rounded-full',
                            ready
                                ? 'bg-success-soft text-success'
                                : failed
                                  ? 'bg-danger-soft text-danger-ink'
                                  : 'bg-accent-soft text-accent-ink',
                        )}
                        aria-hidden="true"
                    >
                        {ready ? (
                            <Check size={30} strokeWidth={2.5} />
                        ) : failed ? (
                            <OctagonX size={28} strokeWidth={2} />
                        ) : (
                            <LoaderCircle
                                size={28}
                                strokeWidth={2}
                                className="animate-spin motion-reduce:animate-none"
                            />
                        )}
                    </span>
                    <h1
                        ref={title}
                        tabIndex={-1}
                        className="text-[32px] leading-tight font-semibold tracking-[-0.02em] text-balance outline-none max-md:text-[26px]"
                    >
                        {ready ? words.online(name) : failed ? words.failed : words.starting}
                    </h1>
                    <output className="max-w-140 text-[15px] leading-relaxed text-ink-2">
                        {status === null ? words.unknown : (cause ?? words.following)}
                    </output>
                </div>

                <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
                    <section aria-label={words.checked} className="card flex flex-col gap-4 p-5 max-md:p-4">
                        <h2 className="text-ui font-medium text-ink">{words.checked}</h2>
                        <Steps
                            label={words.title}
                            steps={LAUNCH_STEPS.map((step, index) => ({
                                key: step,
                                label: words.steps[step],
                                passed: list !== null && index < list.passed,
                                failed: list?.failed === step,
                            }))}
                            current={current}
                            busy={list?.waiting ?? false}
                            said={t.serverSetup.said}
                        />
                        {status !== null && (
                            <p className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-small">
                                <span className={cn('led', STATE_LED[status.state])} aria-hidden="true" />
                                <span className={cn('font-semibold', STATE_INK[status.state])}>
                                    {t.server_state.states[status.state]}
                                </span>
                            </p>
                        )}
                        {status !== null && <HangAdvice status={status} settings={settings} />}
                    </section>
                    <section
                        aria-label={words.consoleTitle}
                        className="card flex flex-col gap-3 p-5 max-md:p-4"
                    >
                        <h2 className="flex items-center gap-2 text-ui font-medium text-ink">
                            <SquareTerminal
                                size={15}
                                strokeWidth={1.75}
                                className="text-ink-3"
                                aria-hidden="true"
                            />
                            {words.consoleTitle}
                        </h2>
                        <p className="text-small text-ink-2">{words.console}</p>
                        <Link
                            to="/console"
                            className="inline-flex items-center gap-1.5 self-start text-ui font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                        >
                            {words.openConsole}
                            <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                        </Link>
                    </section>
                </div>

                {ready && (
                    <section className="flex flex-col gap-3">
                        <h2 className="font-mono text-eyebrow tracking-[0.08em] text-ink-3 uppercase">
                            {words.next}
                        </h2>
                        <ul className="grid grid-cols-1 gap-3 md:grid-cols-3">
                            <li className="flex">
                                <Link
                                    to="/settings/$category"
                                    params={{ category: 'restarts' }}
                                    className="card flex flex-1 flex-col gap-1.5 p-4 transition-colors hover:bg-hover"
                                >
                                    <span className="text-body font-semibold text-ink">
                                        {words.nextRestarts}
                                    </span>
                                    <span className="text-small text-ink-3">{words.nextRestartsAbout}</span>
                                </Link>
                            </li>
                            <li className="flex">
                                <Link
                                    to="/settings/$category"
                                    params={{ category: 'users' }}
                                    className="card flex flex-1 flex-col gap-1.5 p-4 transition-colors hover:bg-hover"
                                >
                                    <span className="text-body font-semibold text-ink">
                                        {words.nextStaff}
                                    </span>
                                    <span className="text-small text-ink-3">{words.nextStaffAbout}</span>
                                </Link>
                            </li>
                            <li className="flex">
                                <Link
                                    to="/backups"
                                    className="card flex flex-1 flex-col gap-1.5 p-4 transition-colors hover:bg-hover"
                                >
                                    <span className="text-body font-semibold text-ink">
                                        {words.nextBackups}
                                    </span>
                                    <span className="text-small text-ink-3">{words.nextBackupsAbout}</span>
                                </Link>
                            </li>
                        </ul>
                    </section>
                )}

                <div className="flex flex-wrap justify-center gap-2.5 max-md:flex-col max-md:items-stretch">
                    {failed ? (
                        <Button tone="ink" icon={ArrowLeft} onClick={onBack}>
                            {words.backToSettings}
                        </Button>
                    ) : (
                        <Link to="/" className={cn(buttonVariants({ tone: ready ? 'ink' : 'secondary' }))}>
                            {words.overview}
                            <ArrowRight size={16} strokeWidth={2} aria-hidden="true" />
                        </Link>
                    )}
                </div>
            </div>
        </div>
    );
}
