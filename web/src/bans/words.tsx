import { Ban as BanIcon, Undo2 } from 'lucide-react';

import type { Ban } from '../api/generated/Ban';
import { Chip } from '../components/Chip';
import { useI18n } from '../i18n/I18nProvider';
import { useNow } from '../lib/useNow';
import { useVisit } from '../players/words';

/** A ban's length, from when it was made to when it ends; `null` for a permanent one. */
export const minutesOf = (ban: Pick<Ban, 'createdAt' | 'endsAt'>) =>
    ban.endsAt === null ? null : Math.round((Date.parse(ban.endsAt) - Date.parse(ban.createdAt)) / 60_000);

/** Its state as a word, with an icon: a colour never carries it alone. */
export function BanStateChip({ ban }: { ban: Ban }) {
    const { t } = useI18n();
    return (
        <Chip
            tone={ban.state === 'active' ? 'danger' : 'neutral'}
            icon={ban.state === 'active' ? BanIcon : ban.state === 'lifted' ? Undo2 : undefined}
        >
            {t.bans.states[ban.state]}
        </Chip>
    );
}

/** When a ban ends, or ended, or who lifted it, in the zone the page reads times in. */
export function BanEnds({ ban, zone }: { ban: Ban; zone: string }) {
    const { t, format } = useI18n();
    const words = t.bans;
    const visit = useVisit(zone);
    const now = useNow(60_000);
    if (ban.lifted !== null) {
        return (
            <div className="flex flex-col gap-0.5">
                <span className="flex flex-wrap items-center gap-1.5">
                    <BanStateChip ban={ban} />
                    <span className="text-meta text-ink-3">{words.liftedBy(ban.lifted.by)}</span>
                </span>
                <span className="font-mono text-small whitespace-nowrap text-ink-3">
                    {visit(ban.lifted.at)}
                </span>
                {ban.lifted.reason !== null && (
                    <span className="max-w-64 text-meta text-ink-3 [overflow-wrap:anywhere]">
                        {ban.lifted.reason}
                    </span>
                )}
            </div>
        );
    }
    if (ban.endsAt === null) {
        return <Chip tone="danger">{words.permanent}</Chip>;
    }
    return (
        <div className="flex flex-col gap-0.5">
            {ban.state === 'ended' ? (
                <span className="flex">
                    <BanStateChip ban={ban} />
                </span>
            ) : (
                <span className="whitespace-nowrap text-ink">
                    {words.endsIn(format.duration((Date.parse(ban.endsAt) - now) / 1000))}
                </span>
            )}
            <span className="font-mono text-small whitespace-nowrap text-ink-3">{visit(ban.endsAt)}</span>
        </div>
    );
}
