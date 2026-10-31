import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Settings2 } from 'lucide-react';
import { useState } from 'react';

import { dropsSettingsQuery, saveDropsSettings, SESSION_DAYS } from '../api/drops';
import type { DropsSettings } from '../api/generated/DropsSettings';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';

type Outcome = { tone: 'success' | 'danger'; message: string };

/** A whole number of days as typed; anything else, an empty field included, is out of range. */
const days = (text: string) => (/^\d+$/.test(text.trim()) ? Number(text.trim()) : Number.NaN);

const within = (value: number) =>
    Number.isInteger(value) && value >= SESSION_DAYS.min && value <= SESSION_DAYS.max;

/** The owner's: how long players' sessions and their texts are kept. */
export function SettingsCard() {
    const { t } = useI18n();
    const words = t.drops.settings;
    const settings = useQuery(dropsSettingsQuery);
    return (
        <Card id="drops-settings" icon={Settings2} title={words.title} meta={words.meta}>
            <div className="flex max-w-2xl flex-col gap-4">
                <p className="text-small text-ink-3">{words.lead}</p>
                {settings.isPending && (
                    <div className="flex flex-col gap-3" aria-busy="true">
                        <span className="sr-only">{words.loading}</span>
                        <span className="skeleton w-1/2" />
                    </div>
                )}
                {settings.isError && (
                    <Notice tone="danger">
                        {words.failed} {errorMessage(t, settings.error)}
                    </Notice>
                )}
                {settings.data !== undefined && <RetentionForm saved={settings.data} />}
            </div>
        </Card>
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
    const changed = draft !== String(saved.sessionDays);
    // The reauthentication is asked by the client when the API wants it, as for every owner's change.
    const save = useMutation({
        mutationFn: () => saveDropsSettings({ sessionDays: days(draft) }),
        onMutate: () => setOutcome(null),
        onSuccess: (stored) => {
            client.setQueryData(dropsSettingsQuery.queryKey, stored);
            setDraft(String(stored.sessionDays));
            setChecked(false);
            setOutcome({ tone: 'success', message: words.saved });
        },
        onError: (error) => setOutcome({ tone: 'danger', message: errorMessage(t, error) }),
    });
    return (
        <form
            className="flex flex-col gap-5"
            noValidate
            onSubmit={(event) => {
                event.preventDefault();
                setChecked(true);
                if (within(days(draft)) && !save.isPending) {
                    save.mutate();
                }
            }}
        >
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            <TextField
                label={words.sessionDays}
                hint={words.hint}
                error={invalid ? words.outOfRange : undefined}
                type="number"
                inputMode="numeric"
                min={SESSION_DAYS.min}
                max={SESSION_DAYS.max}
                className="max-w-md"
                value={draft}
                onChange={(event) => setDraft(event.target.value)}
            />
            {changed && (
                <div className="sticky bottom-3 z-10 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-box border border-line-strong bg-surface px-3.5 py-2.5 shadow-pop">
                    <output className="mr-auto text-ui text-ink-2">{words.changes(1)}</output>
                    <Button
                        tone="ghost"
                        className="max-md:flex-1"
                        onClick={() => {
                            setDraft(String(saved.sessionDays));
                            setChecked(false);
                            setOutcome({ tone: 'success', message: words.discarded });
                        }}
                    >
                        {words.discard}
                    </Button>
                    <Button type="submit" tone="ink" pending={save.isPending} className="max-md:flex-1">
                        {words.save}
                    </Button>
                </div>
            )}
        </form>
    );
}
