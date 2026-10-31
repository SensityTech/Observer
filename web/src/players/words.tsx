import {
    Clock,
    CircleHelp,
    LogOut,
    OctagonX,
    ShieldAlert,
    Square,
    TimerOff,
    UserX,
    type LucideIcon,
} from 'lucide-react';

import type { DropCategory } from '../api/generated/DropCategory';
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
