import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';

import type { OnlinePlayer } from '../api/generated/OnlinePlayer';
import { Permission } from '../api/generated/Permission';
import { onlineQuery } from '../api/players';
import { allows, useMe } from '../auth/me';
import { useI18n } from '../i18n/I18nProvider';

export type Online = ReadonlyMap<number, OnlinePlayer>;

/** The players online by their server id, for an account that sees players; empty for another. */
export const useOnline = (): Online => {
    const me = useMe();
    const sees = allows(me, Permission.PlayersView);
    const online = useQuery({ ...onlineQuery, enabled: sees });
    return new Map((sees ? (online.data?.players ?? []) : []).map((player) => [player.netId, player]));
};

/**
 * A client by its server id, with its name and the way to its profile while it is online. The
 * hook counts by id alone: the name is the panel's.
 */
export function Client({ netId, online }: { netId: number | null; online: Online }) {
    const { t } = useI18n();
    const words = t.debug;
    if (netId === null) {
        return <span className="text-ink-3">{words.events.refusedCard.unknown}</span>;
    }
    const player = online.get(netId);
    const id = <span className="font-mono text-meta text-ink-3">{words.player(netId)}</span>;
    if (player === undefined) {
        return (
            <span className="inline-flex flex-wrap items-baseline gap-x-1.5">
                {id}
                {online.size > 0 && <span className="text-meta text-ink-3">{words.left}</span>}
            </span>
        );
    }
    const name = player.label?.name ?? player.name ?? words.unnamed;
    return (
        <span className="inline-flex min-w-0 flex-wrap items-baseline gap-x-1.5">
            {player.player === null ? (
                <span className="text-ink [overflow-wrap:anywhere]">{name}</span>
            ) : (
                <Link
                    to="/players/$id"
                    params={{ id: String(player.player) }}
                    className="font-medium text-ink underline-offset-2 [overflow-wrap:anywhere] hover:underline"
                >
                    {name}
                </Link>
            )}
            {id}
        </span>
    );
}
