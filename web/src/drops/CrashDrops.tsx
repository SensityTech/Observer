import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Unplug } from 'lucide-react';

import { crashDropsQuery } from '../api/drops';
import type { Crash } from '../api/generated/Crash';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { table } from '../components/table';
import { useI18n } from '../i18n/I18nProvider';
import { useNow } from '../lib/useNow';
import { rangeCovering } from './search';

/**
 * How many players a crash or a hang of FXServer dropped, for who sees drop statistics, and the way
 * to their drops in the log for who sees players. A record of dumps found has no run to ask about.
 */
export function CrashDrops({ crash }: { crash: Crash }) {
    const me = useMe();
    if (
        !allows(me, Permission.DropsView) ||
        crash.kind === 'found' ||
        crash.runStarted === null ||
        crash.generation === null
    ) {
        return null;
    }
    return (
        <Dropped
            at={crash.at}
            started={crash.runStarted}
            generation={crash.generation}
            withLog={allows(me, Permission.PlayersView)}
        />
    );
}

function Dropped({
    at,
    started,
    generation,
    withLog,
}: {
    at: string;
    started: string;
    generation: number;
    withLog: boolean;
}) {
    const { t } = useI18n();
    const now = useNow(60_000);
    const drops = useQuery(crashDropsQuery(started, generation));
    if (drops.data == null) {
        return null;
    }
    const { run, players } = drops.data;
    return (
        <div className={table.sub}>
            <Unplug size={12} strokeWidth={2} aria-hidden="true" />
            <span>{t.drops.crash.dropped(players)}</span>
            {withLog && players > 0 && (
                <Link
                    to="/drops"
                    search={{ range: rangeCovering(at, now), run }}
                    className="inline-flex items-center gap-1 rounded-xs font-medium text-ink-2 underline-offset-2 hover:text-ink hover:underline max-md:min-h-11"
                >
                    {t.drops.crash.showInLog}
                    <ArrowRight size={12} strokeWidth={2} aria-hidden="true" />
                </Link>
            )}
        </div>
    );
}
