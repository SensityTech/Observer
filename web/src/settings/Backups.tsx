import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight } from 'lucide-react';

import { backupsQuery } from '../api/backups';
import type { BackupsView } from '../api/generated/BackupsView';
import type { BeforeRestarts } from '../api/generated/BeforeRestarts';
import { useBackupsSummary } from '../backups/summary';
import { Notice } from '../components/Notice';
import type { Messages } from '../i18n/en';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { CategoryHead, SettingRow, SettingsGroup } from './Row';

const BEFORE_RESTARTS = {
    off: 'off',
    scheduled: 'scheduled',
    allRestarts: 'all',
} satisfies Record<BeforeRestarts, keyof Messages['backups']['when']>;

/**
 * The backups of the game database at a glance, read where the Backups page reads them: they are
 * set, run and restored there, with their own dialogs and permissions.
 */
export function BackupsSettings() {
    const { t } = useI18n();
    const backups = useQuery(backupsQuery);

    return (
        <>
            <CategoryHead
                category="backups"
                aside={
                    <Link
                        to="/backups"
                        className="inline-flex items-center gap-1.5 text-ui font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                    >
                        {t.settings.backups.openBackups}
                        <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                    </Link>
                }
            />
            {backups.isPending && (
                <div className="card flex flex-col gap-3 p-5">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-2/3" />
                    <span className="skeleton w-1/2" />
                </div>
            )}
            {backups.isError && backups.data === undefined && (
                <Notice tone="danger">
                    {t.backups.loadFailed} {errorMessage(t, backups.error)}
                </Notice>
            )}
            {backups.data !== undefined && <Summary view={backups.data} />}
        </>
    );
}

function Summary({ view }: { view: BackupsView }) {
    const { t } = useI18n();
    const words = t.settings.backups;
    const { sentence, next } = useBackupsSummary(view);
    const { beforeRestarts, times } = view.settings;

    return (
        <SettingsGroup title={t.settings.categories.backups.name}>
            <SettingRow id="backupsBeforeRestarts">
                {() => (
                    <p className="text-body text-ink-2">{t.backups.when[BEFORE_RESTARTS[beforeRestarts]]}</p>
                )}
            </SettingRow>
            <SettingRow id="backupsTimes">
                {() =>
                    times.length === 0 ? (
                        <p className="text-body text-ink-3">{words.noTimes}</p>
                    ) : (
                        <p className="font-mono text-ui text-ink">{times.join(', ')}</p>
                    )
                }
            </SettingRow>
            <SettingRow id="backupsDestinations">
                {() =>
                    view.destinations.length === 0 ? (
                        <p className="text-body text-ink-3">{words.noDestination}</p>
                    ) : (
                        <ul className="flex flex-col gap-1 text-body text-ink-2">
                            {view.destinations.map((destination) => (
                                <li key={destination.id}>
                                    {words.destination(destination.name, destination.enabled)}
                                </li>
                            ))}
                        </ul>
                    )
                }
            </SettingRow>
            <SettingRow id="backupsLast">
                {() => (
                    <p className="text-body text-pretty text-ink-2">
                        {sentence} {next}
                    </p>
                )}
            </SettingRow>
        </SettingsGroup>
    );
}
