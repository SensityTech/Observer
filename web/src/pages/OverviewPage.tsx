import { useQuery } from '@tanstack/react-query';

import { healthQuery } from '../api/health';
import { useI18n } from '../i18n/I18nProvider';

export function OverviewPage() {
    const { t } = useI18n();
    const health = useQuery(healthQuery);

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
                            <p className="flex items-center gap-2.5 text-xl font-semibold tracking-tight">
                                <span className="dot is-down" />
                                {t.overview.unreachable}
                            </p>
                            <p className="text-ink-3">{t.overview.unreachableHint}</p>
                        </>
                    )}
                    {health.data && (
                        <>
                            <p className="flex items-center gap-2.5 text-xl font-semibold tracking-tight">
                                <span className="dot is-live" />
                                {t.overview.running}
                            </p>
                            <p className="font-mono text-[13px] text-ink-3">
                                {t.overview.version(health.data.version)}
                            </p>
                        </>
                    )}
                </div>
            </section>
        </>
    );
}
