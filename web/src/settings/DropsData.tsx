import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';

import { allDrops, dropsSettingsQuery, saveDropsSettings, SESSION_DAYS } from '../api/drops';
import type { DropsSettings } from '../api/generated/DropsSettings';
import { FieldError, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useSection } from './Form';
import { GroupNotice, SettingRow, SettingsGroup } from './Row';

type Outcome = { tone: 'success' | 'neutral' | 'danger'; message: string };

/** A whole number of days as typed; anything else, an empty field included, is out of range. */
const days = (text: string) => (/^\d+$/.test(text.trim()) ? Number(text.trim()) : Number.NaN);

const within = (value: number) =>
    Number.isInteger(value) && value >= SESSION_DAYS.min && value <= SESSION_DAYS.max;

/** The owner's: how long players' sessions and their texts are kept. */
export function DropsData() {
    const { t } = useI18n();
    const words = t.drops.settings;
    const settings = useQuery(dropsSettingsQuery);
    return (
        <SettingsGroup title={t.settings.data.drops}>
            {settings.isPending && (
                <GroupNotice>
                    <div className="flex flex-col gap-3 pb-4" aria-busy="true">
                        <span className="sr-only">{words.loading}</span>
                        <span className="skeleton w-1/2" />
                    </div>
                </GroupNotice>
            )}
            {settings.isError && (
                <GroupNotice>
                    <div className="pb-4">
                        <Notice tone="danger">
                            {words.failed} {errorMessage(t, settings.error)}
                        </Notice>
                    </div>
                </GroupNotice>
            )}
            {settings.data !== undefined && <RetentionForm saved={settings.data} />}
        </SettingsGroup>
    );
}

function RetentionForm({ saved }: { saved: DropsSettings }) {
    const { t } = useI18n();
    const words = t.drops.settings;
    const client = useQueryClient();
    const [draft, setDraft] = useState(String(saved.sessionDays));
    const [checked, setChecked] = useState(false);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const invalid = checked && !within(days(draft));
    // The reauthentication is asked by the client when the API wants it, as for every owner's change.
    const save = useMutation({
        mutationFn: () => saveDropsSettings({ sessionDays: days(draft) }),
        onMutate: () => setOutcome(null),
        onSuccess: (stored) => {
            client.setQueryData(dropsSettingsQuery.queryKey, stored);
            setDraft(String(stored.sessionDays));
            setChecked(false);
            setOutcome({ tone: 'success', message: words.saved });
            // What players are counted over follows the sessions kept: every figure is read again.
            void client.invalidateQueries({
                ...allDrops,
                predicate: (query) => query.queryKey[1] !== 'settings',
            });
        },
        onError: (error) => setOutcome({ tone: 'danger', message: errorMessage(t, error) }),
    });

    useSection({
        changed: draft === String(saved.sessionDays) ? [] : ['sessionDays'],
        pending: save.isPending,
        save: () => {
            setChecked(true);
            if (within(days(draft))) {
                save.mutate();
            }
        },
        discard: () => {
            setDraft(String(saved.sessionDays));
            setChecked(false);
            setOutcome({ tone: 'neutral', message: words.discarded });
        },
    });

    return (
        <>
            {outcome !== null && (
                <GroupNotice>
                    <Notice tone={outcome.tone}>{outcome.message}</Notice>
                </GroupNotice>
            )}
            <SettingRow id="sessionDays" field changed={draft !== String(saved.sessionDays)}>
                {(ids) => (
                    <>
                        <input
                            id={ids.control}
                            type="number"
                            inputMode="numeric"
                            min={SESSION_DAYS.min}
                            max={SESSION_DAYS.max}
                            className={cn(inputClass, 'max-w-40')}
                            aria-invalid={invalid || undefined}
                            aria-describedby={invalid ? `${ids.control}-error ${ids.about}` : ids.about}
                            value={draft}
                            onChange={(event) => setDraft(event.target.value)}
                        />
                        {invalid && <FieldError id={`${ids.control}-error`}>{words.outOfRange}</FieldError>}
                    </>
                )}
            </SettingRow>
        </>
    );
}
