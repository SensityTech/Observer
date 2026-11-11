import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';

import { characters } from '../api/bans';
import type { KickSettings as Settings } from '../api/generated/KickSettings';
import { Permission } from '../api/generated/Permission';
import { kickSettingsQuery, MAX_KICK_MESSAGE, saveKickSettings } from '../api/moderation';
import { allows, useMe } from '../auth/me';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { fill } from '../lib/fill';
import { useSection } from './Form';
import { MessageEditor } from './MessageEditor';
import { SettingRow, SettingsGroup } from './Row';

type Outcome = { tone: 'success' | 'danger'; message: string };
const PLACEHOLDERS = ['reason', 'by'] as const;

/** What a kicked player reads: a section of the bans' category, saved through its bar. */
export function KickSettings() {
    const { t } = useI18n();
    const settings = useQuery(kickSettingsQuery);
    if (settings.isPending) {
        return (
            <section className="card">
                <div className="card-body flex flex-col gap-3" aria-busy="true">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-1/3" />
                    <span className="skeleton w-1/2" />
                </div>
            </section>
        );
    }
    if (settings.isError) {
        return (
            <Notice tone="danger">
                {t.settings.bans.loadFailed} {errorMessage(t, settings.error)}
            </Notice>
        );
    }
    return <KickForm stored={settings.data} />;
}

function KickForm({ stored }: { stored: Settings }) {
    const { t } = useI18n();
    const words = t.settings.bans;
    const me = useMe();
    const client = useQueryClient();
    const canManage = allows(me, Permission.SettingsManage);
    const [message, setMessage] = useState(stored.message);
    const [tried, setTried] = useState(false);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const problem =
        message.trim() === ''
            ? words.kickMessageMissing
            : characters(message.trim()) > MAX_KICK_MESSAGE
              ? words.messageTooLong(MAX_KICK_MESSAGE)
              : null;
    const changed = message === stored.message ? [] : (['kickMessage'] as const);
    const save = useMutation({
        mutationFn: () => saveKickSettings({ message }),
        onMutate: () => setOutcome(null),
        onSuccess: (saved) => {
            client.setQueryData(kickSettingsQuery.queryKey, saved);
            setMessage(saved.message);
            setTried(false);
            setOutcome({ tone: 'success', message: words.kickSaved });
        },
        onError: (error) => setOutcome({ tone: 'danger', message: errorMessage(t, error) }),
    });
    useSection({
        changed: canManage ? changed : [],
        pending: save.isPending,
        save: () => {
            setTried(true);
            if (problem === null) {
                save.mutate();
            }
        },
        discard: () => {
            setMessage(stored.message);
            setTried(false);
            setOutcome({ tone: 'success', message: words.discarded });
        },
    });

    return (
        <fieldset disabled={!canManage} className="flex min-w-0 flex-col gap-4">
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            <SettingsGroup title={words.kicks}>
                <SettingRow id="kickMessage" field changed={canManage && changed.length > 0}>
                    {(ids) => (
                        <MessageEditor
                            ids={ids}
                            value={message}
                            onChange={setMessage}
                            placeholders={PLACEHOLDERS}
                            insertLabel={words.insert}
                            problem={tried ? problem : null}
                            previewLabel={words.kickPreview}
                            preview={fill(message.trim(), words.kickExample)}
                        />
                    )}
                </SettingRow>
            </SettingsGroup>
        </fieldset>
    );
}
