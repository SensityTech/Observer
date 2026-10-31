import { useQuery } from '@tanstack/react-query';
import { useRouterState } from '@tanstack/react-router';

import { communityQuery } from '../api/players';
import { useMe } from '../auth/me';
import { Notice } from '../components/Notice';
import { useI18n } from '../i18n/I18nProvider';
import { CommunitySection } from '../players/CommunitySection';
import { DataCard } from '../players/DataCard';
import { OnlineCard } from '../players/OnlineCard';
import { PlayersCard } from '../players/PlayersCard';
import { ZoneNote } from '../players/words';
import { useSchedule } from '../schedule/context';

export function PlayersPage() {
    const { t } = useI18n();
    const me = useMe();
    const forgotten = useRouterState({ select: (state) => state.location.state.forgotten === true });
    // Days are dates in the schedule's zone, which the community figures name; the schedule's own
    // until they arrive.
    const community = useQuery(communityQuery);
    const scheduled = useSchedule().zone;
    const zone = community.data?.zone ?? scheduled ?? 'UTC';
    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.players.title} <span className="gloss">{t.players.gloss}</span>
                </h1>
                <ZoneNote zone={zone} />
            </div>
            {forgotten && <Notice tone="success">{t.players.forgotten}</Notice>}
            <OnlineCard />
            <CommunitySection />
            <PlayersCard zone={zone} />
            {me.isOwner && <DataCard zone={zone} />}
        </div>
    );
}
