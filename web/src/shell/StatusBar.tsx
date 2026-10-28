import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import type { LinkState } from '../live/realtime';
import { cn } from '../lib/cn';
import { secondsSince, useNow } from '../lib/useNow';
import { useServerStatus } from '../server/context';
import { STATE_INK, STATE_LED, isRunning } from '../server/state';

const LINK_LED = {
    connecting: 'led-pending',
    live: 'led-live',
    offline: 'led-down',
    refused: 'led-down',
} satisfies Record<LinkState, string>;

/** FXServer's state, build and players, and the live link's, updated in place (DESIGN.md §7.3). */
export function StatusBar() {
    const { t, format } = useI18n();
    const link = useLinkState();
    const { status } = useServerStatus();
    const now = useNow();
    const unknown = <span className="font-normal text-ink-3">{t.status.unknown}</span>;

    const uptime =
        status !== null && isRunning(status.state) && status.startedAt !== null
            ? t.status.upFor(format.duration(secondsSince(status.startedAt, now)))
            : null;
    const players = status?.players ?? null;

    return (
        <dl className="status">
            <div className="status-cell">
                <dt className="md:sr-only">{t.status.state}</dt>
                <dd>
                    {status === null ? (
                        <>
                            <span className="led led-off" aria-hidden="true" />
                            {unknown}
                        </>
                    ) : (
                        <>
                            <span className={cn('led', STATE_LED[status.state])} aria-hidden="true" />
                            <span className={cn('font-semibold', STATE_INK[status.state])}>
                                {t.server_state.states[status.state]}
                            </span>
                            {uptime !== null && (
                                <span className="font-normal text-ink-3 xl:max-[1261px]:hidden md:max-[941px]:hidden">
                                    · {uptime}
                                </span>
                            )}
                        </>
                    )}
                </dd>
            </div>
            <div className="status-cell">
                <dt>{t.status.build}</dt>
                <dd className="font-mono text-[12.5px]">
                    {status?.build == null ? unknown : String(status.build)}
                </dd>
            </div>
            <div className="status-cell">
                <dt>{t.status.players}</dt>
                <dd className="tabular-nums">
                    {players === null
                        ? unknown
                        : t.status.playerCount(
                              format.number(players.online),
                              players.max === null ? null : format.number(players.max),
                          )}
                </dd>
            </div>
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
