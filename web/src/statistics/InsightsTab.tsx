import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { Segmented } from '../components/Segmented';
import { useI18n } from '../i18n/I18nProvider';
import { FramesCard } from './insights/FramesCard';
import { HoursCard } from './insights/HoursCard';
import { MemoryCard } from './insights/MemoryCard';
import { SessionsCard } from './insights/SessionsCard';
import { StabilityCard } from './insights/StabilityCard';
import { DEFAULT_INSIGHT_RANGE, INSIGHT_RANGES, type StatisticsSearch } from './search';

/** The analyses: frames, restart hours, memory run after run, sessions, stability and its causes. */
export function InsightsTab({
    search,
    go,
}: {
    search: StatisticsSearch;
    go: (next: StatisticsSearch, replace?: boolean) => void;
}) {
    const { t } = useI18n();
    const me = useMe();
    const range = search.insights ?? DEFAULT_INSIGHT_RANGE;
    const drops = allows(me, Permission.DropsView);
    return (
        <div className="flex flex-col gap-3">
            <Segmented
                label={t.statistics.insights.range}
                choices={INSIGHT_RANGES}
                value={range}
                words={t.drops.ranges}
                onChange={(choice) =>
                    go({ ...search, insights: choice === DEFAULT_INSIGHT_RANGE ? undefined : choice }, true)
                }
                className="self-start"
            />
            <div className="grid grid-cols-1 gap-3 xl:grid-cols-2">
                <FramesCard range={range} />
                <HoursCard />
                <MemoryCard range={range} />
                {drops && <SessionsCard range={range} />}
            </div>
            {drops ? <StabilityCard /> : <p className="text-small text-ink-3">{t.statistics.needsDrops}</p>}
        </div>
    );
}
