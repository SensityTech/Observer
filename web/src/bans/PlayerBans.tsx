import { useQuery } from '@tanstack/react-query';
import { Ban as BanIcon, Pencil, Undo2 } from 'lucide-react';
import { useState } from 'react';

import { playerBansQuery } from '../api/bans';
import type { Ban } from '../api/generated/Ban';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useNow } from '../lib/useNow';
import { useVisit } from '../players/words';
import { EditDialog, LiftDialog } from './dialogs';
import { BanEdits, BanStateChip, minutesOf } from './words';

/** The bans of a profile: the one in force said at the top, every one listed below. */
export function PlayerBans({ player, zone }: { player: number; zone: string }) {
    const me = useMe();
    const bans = useQuery(playerBansQuery(player));
    const [editing, setEditing] = useState<Ban | null>(null);
    const [lifting, setLifting] = useState<Ban | null>(null);
    if (!bans.isSuccess || bans.data.length === 0) {
        return null;
    }
    const active = bans.data.find((ban) => ban.state === 'active');
    return (
        <>
            {active !== undefined && (
                <ActiveBan
                    ban={active}
                    zone={zone}
                    onEdit={allows(me, Permission.BansEdit) ? () => setEditing(active) : undefined}
                    onLift={allows(me, Permission.BansLift) ? () => setLifting(active) : undefined}
                />
            )}
            <BansCard bans={bans.data} zone={zone} />
            {editing !== null && (
                <EditDialog
                    ban={editing}
                    zone={zone}
                    onClose={() => setEditing(null)}
                    onEdited={() => setEditing(null)}
                />
            )}
            {lifting !== null && (
                <LiftDialog
                    ban={lifting}
                    onClose={() => setLifting(null)}
                    onLifted={() => setLifting(null)}
                />
            )}
        </>
    );
}

function ActiveBan({
    ban,
    zone,
    onEdit,
    onLift,
}: {
    ban: Ban;
    zone: string;
    onEdit?: () => void;
    onLift?: () => void;
}) {
    const { t, format } = useI18n();
    const words = t.bans.profile;
    const visit = useVisit(zone);
    const now = useNow(60_000);
    return (
        <Notice tone="danger">
            <span className="flex min-w-0 flex-1 flex-wrap items-start justify-between gap-x-4 gap-y-2">
                <span className="flex min-w-0 flex-col gap-0.5">
                    <strong className="font-semibold">
                        {ban.endsAt === null
                            ? words.forGood
                            : words.until(
                                  visit(ban.endsAt),
                                  format.duration((Date.parse(ban.endsAt) - now) / 1000),
                              )}
                    </strong>
                    <span className="text-ink-2 [overflow-wrap:anywhere]">{ban.reason}</span>
                    <span className="text-ink-3">{words.madeBy(ban.by, visit(ban.createdAt), ban.code)}</span>
                    <span className="text-ink-3">{t.bans.refused(ban.refused)}</span>
                </span>
                {(onEdit !== undefined || onLift !== undefined) && (
                    <span className="flex flex-wrap gap-2">
                        {onEdit !== undefined && (
                            <Button size="sm" icon={Pencil} onClick={onEdit}>
                                {t.bans.edit}
                            </Button>
                        )}
                        {onLift !== undefined && (
                            <Button size="sm" icon={Undo2} onClick={onLift}>
                                {t.bans.lift}
                            </Button>
                        )}
                    </span>
                )}
            </span>
        </Notice>
    );
}

function BansCard({ bans, zone }: { bans: Ban[]; zone: string }) {
    const { t } = useI18n();
    const words = t.bans;
    const visit = useVisit(zone);
    const active = bans.filter((ban) => ban.state === 'active').length;
    return (
        <Card
            id="player-bans"
            icon={BanIcon}
            title={words.profile.title}
            meta={words.profile.meta(bans.length, active)}
            flush
        >
            <div className={cn(table.scroll, 'max-md:hidden')}>
                <table className={table.table}>
                    <thead>
                        <tr>
                            <th scope="col" className={table.th}>
                                {words.code}
                            </th>
                            <th scope="col" className={table.th}>
                                {words.reason}
                            </th>
                            <th scope="col" className={table.th}>
                                {words.profile.length}
                            </th>
                            <th scope="col" className={table.th}>
                                {words.profile.state}
                            </th>
                        </tr>
                    </thead>
                    <tbody>
                        {bans.map((ban) => (
                            <tr key={ban.code} className={table.row}>
                                <td className={cn(table.td, 'align-top')}>
                                    <span className="font-mono text-small text-ink">{ban.code}</span>
                                    <div className={table.sub}>
                                        {ban.by} · {visit(ban.createdAt)}
                                    </div>
                                </td>
                                <td className={cn(table.td, 'max-w-96 align-top [overflow-wrap:anywhere]')}>
                                    {ban.reason}
                                    <BanEdits ban={ban} zone={zone} />
                                </td>
                                <td className={cn(table.td, 'align-top whitespace-nowrap')}>
                                    {words.lengthOf(minutesOf(ban))}
                                </td>
                                <td className={cn(table.td, 'align-top')}>
                                    <BanStateChip ban={ban} />
                                    {ban.lifted !== null && (
                                        <div className={table.sub}>
                                            {words.liftedBy(ban.lifted.by)}
                                            {ban.lifted.reason !== null && `: ${ban.lifted.reason}`}
                                        </div>
                                    )}
                                </td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            </div>
            <ul className="flex flex-col md:hidden">
                {bans.map((ban) => (
                    <li
                        key={ban.code}
                        className="flex flex-col gap-1.5 border-b border-line px-4 py-3 last:border-b-0"
                    >
                        <div className="flex flex-wrap items-center justify-between gap-2">
                            <span className="font-mono text-small text-ink">{ban.code}</span>
                            <BanStateChip ban={ban} />
                        </div>
                        <p className="text-ui text-ink-2 [overflow-wrap:anywhere]">{ban.reason}</p>
                        <BanEdits ban={ban} zone={zone} />
                        <p className="text-meta text-ink-3">
                            {words.lengthOf(minutesOf(ban))} · {ban.by} · {visit(ban.createdAt)}
                        </p>
                    </li>
                ))}
            </ul>
        </Card>
    );
}
