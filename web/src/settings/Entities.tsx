import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';

import { CADENCES, entitiesSettingsQuery, saveEntitiesSettings } from '../api/entities';
import type { EntitiesSettings as Settings } from '../api/generated/EntitiesSettings';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { SettingsForm, useSection } from './Form';
import { CategoryHead, CategoryLink, CategoryLoading, Segmented, SettingRow, SettingsGroup } from './Row';

type Outcome = { tone: 'success' | 'danger'; message: string };

/** How often the server lists its entities while someone looks at them. */
export function EntitiesSettings() {
    const { t } = useI18n();
    const me = useMe();
    const settings = useQuery(entitiesSettingsQuery);
    return (
        <>
            <CategoryHead
                category="entities"
                aside={
                    allows(me, Permission.EntitiesView) ? (
                        <CategoryLink to="/entities">{t.settings.entities.openEntities}</CategoryLink>
                    ) : undefined
                }
            />
            {settings.isPending && <CategoryLoading />}
            {settings.isError && (
                <Notice tone="danger">
                    {t.settings.entities.loadFailed} {errorMessage(t, settings.error)}
                </Notice>
            )}
            {settings.isSuccess && (
                <SettingsForm>
                    <EntitiesForm stored={settings.data} />
                </SettingsForm>
            )}
        </>
    );
}

function EntitiesForm({ stored }: { stored: Settings }) {
    const { t } = useI18n();
    const words = t.settings.entities;
    const me = useMe();
    const client = useQueryClient();
    const canManage = allows(me, Permission.SettingsManage);
    const [every, setEvery] = useState(stored.everySeconds);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const save = useMutation({
        mutationFn: () => saveEntitiesSettings({ everySeconds: every }),
        onMutate: () => setOutcome(null),
        onSuccess: (saved) => {
            client.setQueryData(entitiesSettingsQuery.queryKey, saved);
            setEvery(saved.everySeconds);
            setOutcome({ tone: 'success', message: words.saved });
        },
        onError: (error) => setOutcome({ tone: 'danger', message: errorMessage(t, error) }),
    });
    const changed = every !== stored.everySeconds;
    useSection({
        changed: canManage && changed ? ['entitiesCadence'] : [],
        pending: save.isPending,
        save: () => save.mutate(),
        discard: () => {
            setEvery(stored.everySeconds);
            setOutcome({ tone: 'success', message: words.discarded });
        },
    });

    return (
        <fieldset disabled={!canManage} className="flex min-w-0 flex-col gap-4">
            {!canManage && <Notice>{words.readOnly}</Notice>}
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            <SettingsGroup title={t.settings.categories.entities.name}>
                <SettingRow id="entitiesCadence" changed={canManage && changed}>
                    {(ids) => (
                        <Segmented
                            ids={ids}
                            name="entitiesCadence"
                            choices={CADENCES.map(String)}
                            words={Object.fromEntries(
                                CADENCES.map((value) => [String(value), words.seconds(value)]),
                            )}
                            value={String(every)}
                            onChange={(value) => setEvery(Number(value))}
                        />
                    )}
                </SettingRow>
            </SettingsGroup>
        </fieldset>
    );
}
