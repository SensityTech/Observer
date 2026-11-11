import { Ban as BanIcon, ChevronDown, Pencil, Undo2 } from 'lucide-react';

import type { Ban } from '../api/generated/Ban';
import { Chip } from '../components/Chip';
import { useI18n } from '../i18n/I18nProvider';
import { useNow } from '../lib/useNow';
import { useVisit } from '../players/words';

/**
 * A ban's length as it was last set: from when it was made to when it ends, or from the edit that
 * gave it its end; `null` for a permanent one.
 */
export const minutesOf = (ban: Pick<Ban, 'createdAt' | 'endsAt' | 'edits'>) => {
    if (ban.endsAt === null) {
        return null;
    }
    const since = ban.edits.find((edit) => edit.ends !== null)?.at ?? ban.createdAt;
    return Math.round((Date.parse(ban.endsAt) - Date.parse(since)) / 60_000);
};

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

/**
 * What was changed on a ban since it was made, folded under its reason: who changed it, when, and
 * what it said before. Nothing for a ban no one edited.
 */
export function BanEdits({ ban, zone }: { ban: Ban; zone: string }) {
    const { t } = useI18n();
    const words = t.bans.edits;
    const visit = useVisit(zone);
    const latest = ban.edits[0];
    if (latest === undefined) {
        return null;
    }
    const end = (at: string | null) => (at === null ? words.forGood : visit(at));
    return (
        <details className="group">
            <summary className="inline-flex cursor-pointer list-none items-center gap-1 rounded-xs text-meta text-ink-3 underline-offset-2 hover:text-ink-2 hover:underline max-md:min-h-11 [&::-webkit-details-marker]:hidden">
                <Pencil size={11} strokeWidth={1.75} aria-hidden="true" />
                {words.summary(ban.edits.length, latest.by)}
                <ChevronDown
                    size={12}
                    strokeWidth={1.75}
                    aria-hidden="true"
                    className="transition-transform group-open:rotate-180"
                />
            </summary>
            <ol className="mt-1 flex flex-col gap-1.5 text-meta text-ink-3">
                {ban.edits.map((edit) => (
                    <li key={edit.at} className="flex flex-col gap-0.5">
                        <span className="text-ink-2">{words.by(edit.by, visit(edit.at))}</span>
                        {edit.reason !== null && (
                            <span className="[overflow-wrap:anywhere]">
                                {words.reason(edit.reason.before)}
                            </span>
                        )}
                        {edit.ends !== null && (
                            <span>{words.ends(end(edit.ends.before), end(edit.ends.after))}</span>
                        )}
                    </li>
                ))}
            </ol>
        </details>
    );
}
