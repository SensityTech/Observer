import { useQuery } from '@tanstack/react-query';
import { useEffect, useState } from 'react';

import { healthQuery } from '../api/health';
import { useI18n } from '../i18n/I18nProvider';
import { useSnapshot } from '../live/context';

// Uptime reads in seconds for its first minute.
const TICK = 1_000;

export function OverviewPage() {
    const { t, format } = useI18n();
    const health = useQuery(healthQuery);
    const status = useSnapshot('status');
    const [now, setNow] = useState(Date.now);

    useEffect(() => {
        const timer = setInterval(() => setNow(Date.now()), TICK);
        return () => clearInterval(timer);
    }, []);

    return (
        <>
            <div className="page-head">
                <h1 className="page-title">
                    {t.overview.title} <span className="gloss">{t.overview.gloss}</span>
                </h1>
            </div>

            <section className="card" aria-labelledby="overview-state">
                <div className="card-head">
                    <h2 id="overview-state" className="eyebrow">
                        {t.overview.stateTitle}
                    </h2>
                </div>
                <div className="card-body flex flex-col gap-1.5">
                    {health.isPending && <p className="text-ink-3">{t.overview.loading}</p>}
                    {health.isError && (
                        <>
                            <p className="flex items-center gap-2.5 text-state text-ink">
                                <span className="led led-down" aria-hidden="true" />
                                {t.overview.unreachable}
                            </p>
                            <p className="text-ink-3">{t.overview.unreachableHint}</p>
                        </>
                    )}
                    {health.data && (
                        <>
                            <p className="flex items-center gap-2.5 text-state text-ink">
                                <span className="led led-live" aria-hidden="true" />
                                {t.overview.running}
                            </p>
                            <p className="font-mono text-[13px] text-ink-3">
                                {t.overview.version(health.data.version)}
                            </p>
                            {status && (
                                <p className="text-small text-ink-3">
                                    {t.overview.started(
                                        format.timestamp(status.startedAt, { seconds: false }),
                                        // A clock behind the host's never reads as a start in the future.
                                        format.duration(
                                            Math.max(0, now - Date.parse(status.startedAt)) / 1000,
                                        ),
                                    )}
                                </p>
                            )}
                        </>
                    )}
                </div>
            </section>
        </>
    );
}
