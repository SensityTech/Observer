import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { Radio } from 'lucide-react';
import type { ReactNode } from 'react';

import type { OnlinePlayer } from '../api/generated/OnlinePlayer';
import { onlineQuery, shortLicence } from '../api/players';
import { Card } from '../components/Card';
import { CopyValue } from '../components/CopyValue';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

/** Who is connected now: a confirmed player by name and licence, the others by their state only. */
export function OnlineCard() {
    const { t } = useI18n();
    const words = t.players.now;
    const online = useQuery(onlineQuery);
    const players = online.data?.players ?? [];
    return (
        <Card
            id="players-online"
            icon={Radio}
            title={words.title}
            meta={online.data === undefined ? undefined : words.meta(players.length)}
            flush
        >
            {online.isPending && (
                <div className="flex flex-col gap-3 p-5 max-md:p-4" aria-busy="true">
                    <span className="sr-only">{words.loading}</span>
                    <span className="skeleton w-1/2" />
                    <span className="skeleton w-1/3" />
                </div>
            )}
            {online.isError && online.data === undefined && (
                <div className="p-5 max-md:p-4">
                    <Notice tone="danger">{words.failed}</Notice>
                </div>
            )}
            {online.data !== undefined && players.length === 0 && (
                <p className="px-6 py-4 text-center text-ui text-ink-3">{words.empty}</p>
            )}
            {players.length > 0 && (
                <>
                    <div className={cn(table.scroll, 'max-md:hidden')}>
                        <table className={table.table}>
                            <caption className="sr-only">{words.caption}</caption>
                            <thead>
                                <tr>
                                    <th scope="col" className={table.th}>
                                        {words.player}
                                    </th>
                                    <th scope="col" className={table.th}>
                                        {words.licence}
                                    </th>
                                    <th scope="col" className={cn(table.th, 'text-right')}>
                                        {words.session}
                                    </th>
                                    <th scope="col" className={cn(table.th, 'text-right')}>
                                        {words.total}
                                    </th>
                                </tr>
                            </thead>
                            <tbody>
                                {players.map((player) => (
                                    <OnlineRow key={player.netId} player={player} />
                                ))}
                            </tbody>
                        </table>
                    </div>
                    <ul className="flex flex-col md:hidden">
                        {players.map((player) => (
                            <OnlineItem key={player.netId} player={player} />
                        ))}
                    </ul>
                </>
            )}
        </Card>
    );
}

/** The player's name opening their profile, or what is known of a player not confirmed. */
function Who({ player }: { player: OnlinePlayer }) {
    const { t } = useI18n();
    const words = t.players.now;
    if (player.confirmation === 'confirmed' && player.player !== null && player.name !== null) {
        return (
            <Link
                to="/players/$id"
                params={{ id: String(player.player) }}
                className="font-medium text-ink underline-offset-2 [overflow-wrap:anywhere] hover:underline"
            >
                {player.name}
            </Link>
        );
    }
    const confirming = player.confirmation === 'confirming';
    return (
        <span
            className="inline-flex items-center gap-2"
            title={confirming ? words.confirmingHint : words.unconfirmedHint}
        >
            <span className={confirming ? 'led led-pending' : 'led led-off'} aria-hidden="true" />
            <span className="text-ink-2">{confirming ? words.confirming : words.unconfirmed}</span>
            <span className="sr-only">{confirming ? words.confirmingHint : words.unconfirmedHint}</span>
        </span>
    );
}

function Licence({ player }: { player: OnlinePlayer }) {
    const { t } = useI18n();
    return player.license === null ? null : (
        <CopyValue
            value={player.license}
            shown={shortLicence(player.license)}
            label={t.players.copyLicence(player.license)}
        />
    );
}

function OnlineRow({ player }: { player: OnlinePlayer }) {
    const { t, format } = useI18n();
    return (
        <tr className={table.row}>
            <td className={table.td}>
                <Who player={player} />
                <div className={cn(table.sub, 'font-mono')}>{t.players.now.serverId(player.netId)}</div>
            </td>
            <td className={table.td}>
                <Licence player={player} />
            </td>
            <td className={cn(table.td, 'text-right whitespace-nowrap')}>
                {format.playTime(player.sessionMs)}
            </td>
            <td className={cn(table.td, 'text-right font-semibold whitespace-nowrap text-ink')}>
                {player.totalMs === null ? '' : format.playTime(player.totalMs)}
            </td>
        </tr>
    );
}

function OnlineItem({ player }: { player: OnlinePlayer }) {
    const { t, format } = useI18n();
    const words = t.players.now;
    return (
        <li className="flex flex-col gap-1.5 border-b border-line px-4 py-3 last:border-b-0">
            <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
                <Who player={player} />
                <span className="font-mono text-meta text-ink-3">{words.serverId(player.netId)}</span>
            </div>
            <Licence player={player} />
            <dl className="flex flex-wrap gap-x-5 gap-y-1 text-small">
                <Figure term={words.session}>{format.playTime(player.sessionMs)}</Figure>
                {player.totalMs !== null && (
                    <Figure term={words.total}>{format.playTime(player.totalMs)}</Figure>
                )}
            </dl>
        </li>
    );
}

function Figure({ term, children }: { term: string; children: ReactNode }) {
    return (
        <div className="flex items-baseline gap-1.5">
            <dt className="text-ink-3">{term}</dt>
            <dd className="font-medium text-ink">{children}</dd>
        </div>
    );
}
