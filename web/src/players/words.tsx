import { Link } from '@tanstack/react-router';
import {
    Clock,
    CircleHelp,
    LogOut,
    OctagonX,
    ShieldAlert,
    Square,
    Tag,
    TimerOff,
    UserX,
    type LucideIcon,
} from 'lucide-react';

import type { DropCategory } from '../api/generated/DropCategory';
import type { PlayerLabel } from '../api/generated/PlayerLabel';
import { chipVariants } from '../components/Chip';
import { Toggletip } from '../components/Toggletip';
import { formatClock, knownZone, zoneDate, zoneOffset } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { useNow } from '../lib/useNow';

/** A player's state: the LED says it again, the word carries it. Never FXServer's live LED. */
export function OnlineState({ online }: { online: boolean }) {
    const { t } = useI18n();
    return (
        <span className="inline-flex items-center gap-2 whitespace-nowrap">
            <span className={online ? 'led led-ok' : 'led'} aria-hidden="true" />
            <span className={online ? 'text-ink' : 'text-ink-2'}>
                {online ? t.players.online : t.players.offline}
            </span>
        </span>
    );
}

/** A name as the player chose it, in plain text; one left empty by its cleaning says so in grey. */
export function PlayerName({ name }: { name: string }) {
    const { t } = useI18n();
    return name === '' ? <span className="text-ink-3">{t.players.noName}</span> : name;
}

/**
 * The mark of a label the server's resources gave: its ID as a chip, or the tag alone, whose tip
 * says who gave it. A label is their word for the player, never Observer's.
 */
export function LabelChip({ label }: { label: PlayerLabel }) {
    const { t } = useI18n();
    const words = t.players.label;
    return (
        <Toggletip className={chipVariants({ tone: 'neutral' })} tip={words.given(label.resource)}>
            <Tag size={12} strokeWidth={2} aria-hidden="true" />
            {label.id === null ? (
                <span className="sr-only">{words.mark}</span>
            ) : (
                <span className="font-mono">{label.id}</span>
            )}
        </Toggletip>
    );
}

/**
 * A player as the panel names them, opening their profile when it is written: by the label's name
 * when the server's resources gave one, with its chip and the FiveM name under it; by the FiveM name
 * otherwise.
 */
export function ShownName({
    name,
    label,
    player,
}: {
    name: string;
    label: PlayerLabel | null;
    player: number | null;
}) {
    const { t } = useI18n();
    const labelName = label?.name ?? null;
    const shown = labelName ?? <PlayerName name={name} />;
    return (
        <span className="flex min-w-0 flex-col gap-0.5">
            <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
                {player === null ? (
                    <span className="font-medium text-ink [overflow-wrap:anywhere]">{shown}</span>
                ) : (
                    <Link
                        to="/players/$id"
                        params={{ id: String(player) }}
                        className="font-medium text-ink underline-offset-2 [overflow-wrap:anywhere] hover:underline"
                    >
                        {shown}
                    </Link>
                )}
                {label !== null && <LabelChip label={label} />}
            </span>
            {labelName !== null && (
                <span className="text-meta text-ink-3 [overflow-wrap:anywhere]">
                    {t.players.label.fivem(name === '' ? t.players.noName : name)}
                </span>
            )}
        </span>
    );
}

/** The zone the page's days are dates in, and its times read in. */
export function ZoneNote({ zone }: { zone: string }) {
    const { t, locale } = useI18n();
    const now = useNow(60_000);
    return (
        <span className="inline-flex items-center gap-1.5 text-ui text-ink-3">
            <Clock size={14} strokeWidth={1.5} aria-hidden="true" />
            {knownZone(zone) === zone
                ? t.players.zone(zone, zoneOffset(locale, zone, now))
                : t.players.zoneUnknown(zone)}
        </span>
    );
}

/** A visit as tables write times, in the zone the days are in: `2026-10-28 14:02`. */
export const useVisit = (zone: string) => {
    const { locale } = useI18n();
    return (instant: string) => `${zoneDate(instant, zone)} ${formatClock(locale, instant, zone)}`;
};

export const DROP_ICONS = {
    quit: LogOut,
    kick: UserX,
    timeout: TimerOff,
    crash: OctagonX,
    security: ShieldAlert,
    serverStop: Square,
    serverCrash: OctagonX,
    unknown: CircleHelp,
} satisfies Record<DropCategory, LucideIcon>;
