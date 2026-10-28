import { useQuery } from '@tanstack/react-query';

import { healthQuery } from '../api/health';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import type { LinkState } from '../live/realtime';
import { cn } from '../lib/cn';

const LINK_LED = {
    connecting: 'led-pending',
    live: 'led-live',
    offline: 'led-down',
    refused: 'led-down',
} satisfies Record<LinkState, string>;

/** Observer's own state and the live link's for now; FXServer's joins them once Observer supervises a server. */
export function StatusBar() {
    const { t } = useI18n();
    const health = useQuery(healthQuery);
    const link = useLinkState();

    const state = health.isPending ? 'pending' : health.isError ? 'down' : 'ok';

    return (
        <dl className="status" aria-live="polite">
            <div className="status-cell">
                <dt>{t.status.observer}</dt>
                <dd>
                    <span
                        className={cn('led', state === 'ok' && 'led-live', state === 'down' && 'led-down')}
                        aria-hidden="true"
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
            <div className="status-cell">
                <dt>{t.status.updates}</dt>
                <dd>
                    <span className={cn('led', LINK_LED[link])} aria-hidden="true" />
                    {t.status.link[link]}
                </dd>
            </div>
        </dl>
    );
}
