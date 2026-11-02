import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight } from 'lucide-react';

import { observerSettingsQuery } from '../api/settings';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { ChangesData } from './ChangesData';
import { DropsData } from './DropsData';
import { SettingsForm } from './Form';
import { PlayersData } from './PlayersData';
import { CategoryHead, GroupNotice, SettingRow, SettingsGroup } from './Row';

const DAY = 86_400;

/**
 * What Observer records and how long it keeps it, the owner's: what sat at the bottom of the
 * Changes, Drops and Players pages, and the statistics' retention, which nothing changes.
 */
export function DataSettings() {
    const { t } = useI18n();
    return (
        <>
            <CategoryHead
                category="data"
                aside={
                    <Link
                        to="/changes"
                        className="inline-flex items-center gap-1.5 text-ui font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                    >
                        {t.settings.data.openChanges}
                        <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                    </Link>
                }
            />
            <SettingsForm>
                <ChangesData />
                <DropsData />
                <PlayersData />
                <StatisticsKept />
            </SettingsForm>
        </>
    );
}

function StatisticsKept() {
    const { t } = useI18n();
    const words = t.settings.data;
    const settings = useQuery(observerSettingsQuery);
    return (
        <SettingsGroup title={words.statistics}>
            {settings.isError && (
                <GroupNotice>
                    <Notice tone="danger">
                        {words.loadFailed} {errorMessage(t, settings.error)}
                    </Notice>
                </GroupNotice>
            )}
            <SettingRow id="statisticsKept">
                {() =>
                    settings.data === undefined ? (
                        <span className="skeleton w-2/3" aria-busy={settings.isPending} />
                    ) : (
                        <ul className="flex flex-col gap-1 text-body text-ink-2">
                            <li>{words.samples(Math.round(settings.data.statistics.samples / DAY))}</li>
                            <li>{words.minutes(Math.round(settings.data.statistics.minutes / DAY))}</li>
                            <li>{words.hours}</li>
                        </ul>
                    )
                }
            </SettingRow>
        </SettingsGroup>
    );
}
