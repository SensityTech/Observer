import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Rocket } from 'lucide-react';
import { useId, useState } from 'react';

import type { LaunchSettings } from '../api/generated/LaunchSettings';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { launchQuery } from '../api/launch';
import { Card } from '../components/Card';
import { Steps } from '../components/Steps';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useServerStatus } from '../server/context';
import { HangAdvice } from '../server/HangAdvice';
import { STATE_INK, STATE_LED, causeText } from '../server/state';
import { LAUNCH_STEPS, checklist } from './state';

/**
 * The launch the setup asked for, followed on `server.status` from the status before it. Once it
 * is ready or failed it stays as it ended: a restart after a crash is another launch.
 */
export function FirstLaunch({ before }: { before: ServerStatus | null }) {
    const { t } = useI18n();
    const id = useId();
    const words = t.serverSetup.launch;
    const { status: live } = useServerStatus();
    const launch = useQuery(launchQuery);
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
    const current =
        list === null
            ? -1
            : list.failed !== null
              ? LAUNCH_STEPS.indexOf(list.failed)
              : list.waiting
                ? list.passed
                : -1;
    const cause =
        status === null ? null : list?.passed === LAUNCH_STEPS.length ? words.ready : causeText(t, status);

    return (
        <Card id={id} icon={Rocket} title={words.title}>
            <div className="flex flex-col gap-4">
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
                <output className="flex flex-wrap items-center gap-x-2.5 gap-y-1 text-body">
                    {status === null ? (
                        <span className="text-ink-3">{words.unknown}</span>
                    ) : (
                        <>
                            <span className={cn('led', STATE_LED[status.state])} aria-hidden="true" />
                            <span className={cn('font-semibold', STATE_INK[status.state])}>
                                {t.server_state.states[status.state]}.
                            </span>
                            {cause !== null && <span className="text-ink-3">{cause}</span>}
                        </>
                    )}
                </output>
                {status !== null && <HangAdvice status={status} settings={settings} />}
                <div className="flex flex-col items-start gap-2 text-small text-ink-2">
                    <p>{words.console}</p>
                    <Link
                        to="/console"
                        className="inline-flex items-center gap-1.5 font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                    >
                        {words.openConsole}
                        <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                    </Link>
                </div>
            </div>
        </Card>
    );
}
