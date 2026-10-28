import { useQuery } from '@tanstack/react-query';

import { healthQuery } from '../api/health';
import { useI18n } from '../i18n/I18nProvider';

/** Observer's own state for now; FXServer's joins it once Observer supervises a server. */
export function StatusBar() {
    const { t } = useI18n();
    const health = useQuery(healthQuery);

    const state = health.isPending ? 'pending' : health.isError ? 'down' : 'ok';

    return (
        <dl className="status" aria-live="polite">
            <div className="status-cell">
                <dt>{t.status.observer}</dt>
                <dd>
                    <span
                        className={`dot ${state === 'ok' ? 'is-live' : state === 'down' ? 'is-down' : ''}`}
                    />
                    {t.status[state]}
                </dd>
            </div>
            {health.data && (
                <div className="status-cell">
                    <dt>{t.status.version}</dt>
                    <dd className="font-mono">{health.data.version}</dd>
                </div>
            )}
        </dl>
    );
}
