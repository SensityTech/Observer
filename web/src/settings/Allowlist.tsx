import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight } from 'lucide-react';
import { useState } from 'react';

import { useAdmission } from '../admission/context';
import { allowlistSettingsQuery, saveAllowlistSettings } from '../api/allowlist';
import { characters, MAX_MESSAGE } from '../api/bans';
import type { AllowlistSettings as Settings } from '../api/generated/AllowlistSettings';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { Notice } from '../components/Notice';
import { Switch } from '../components/Switch';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { SettingsForm, useSection } from './Form';
import { MessageField } from './MessageField';
import { CategoryHead, SettingRow, SettingsGroup } from './Row';

type Outcome = { tone: 'success' | 'danger'; message: string };
const PLACEHOLDERS = ['id', 'name'] as const;

/** Whether the server is open to approved players only, and what the others read. */
export function AllowlistSettings() {
    const { t } = useI18n();
    const me = useMe();
    const settings = useQuery(allowlistSettingsQuery);
    return (
        <>
            <CategoryHead
                category="allowlist"
                aside={
                    allows(me, Permission.PlayersView) ? (
                        <Link
                            to="/allowlist"
                            className="inline-flex items-center gap-1.5 text-ui font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                        >
                            {t.settings.allowlist.openAllowlist}
                            <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                        </Link>
                    ) : undefined
                }
            />
            {settings.isPending && (
                <section className="card">
                    <div className="card-body flex flex-col gap-3" aria-busy="true">
                        <span className="sr-only">{t.common.loading}</span>
                        <span className="skeleton w-1/3" />
                        <span className="skeleton w-1/2" />
                    </div>
                </section>
            )}
            {settings.isError && (
                <Notice tone="danger">
                    {t.settings.allowlist.loadFailed} {errorMessage(t, settings.error)}
                </Notice>
            )}
            {settings.isSuccess && (
                <SettingsForm>
                    <AllowlistForm stored={settings.data} />
                </SettingsForm>
            )}
        </>
    );
}

function AllowlistForm({ stored }: { stored: Settings }) {
    const { t } = useI18n();
    const words = t.settings.allowlist;
    const me = useMe();
    const client = useQueryClient();
    const admission = useAdmission();
    const canManage = allows(me, Permission.SettingsManage);
    const [draft, setDraft] = useState(stored);
    const [tried, setTried] = useState(false);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const messageProblem =
        draft.message.trim() === ''
            ? words.messageMissing
            : characters(draft.message.trim()) > MAX_MESSAGE
              ? words.messageTooLong(MAX_MESSAGE)
              : null;
    const changed = [
        ...(draft.enabled !== stored.enabled ? (['allowlistEnabled'] as const) : []),
        ...(draft.message !== stored.message ? (['allowlistMessage'] as const) : []),
    ];
    const save = useMutation({
        mutationFn: () => saveAllowlistSettings(draft),
        onMutate: () => setOutcome(null),
        onSuccess: (saved) => {
            client.setQueryData(allowlistSettingsQuery.queryKey, saved);
            setDraft(saved);
            setTried(false);
            setOutcome({ tone: 'success', message: words.saved });
        },
        onError: (error) => setOutcome({ tone: 'danger', message: errorMessage(t, error) }),
    });
    useSection({
        changed: canManage ? changed : [],
        pending: save.isPending,
        save: () => {
            setTried(true);
            if (messageProblem === null) {
                save.mutate();
            }
        },
        discard: () => {
            setDraft(stored);
            setTried(false);
            setOutcome({ tone: 'success', message: words.discarded });
        },
    });

    return (
        <fieldset disabled={!canManage} className="flex min-w-0 flex-col gap-4">
            {!canManage && <Notice>{words.readOnly}</Notice>}
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            <SettingsGroup title={t.settings.categories.allowlist.name}>
                <SettingRow
                    id="allowlistEnabled"
                    field
                    changed={canManage && changed.includes('allowlistEnabled')}
                >
                    {(ids) => (
                        <>
                            <div className="flex items-center gap-2.5">
                                <Switch
                                    id={ids.control}
                                    aria-describedby={ids.about}
                                    checked={draft.enabled}
                                    onChange={(event) =>
                                        setDraft({ ...draft, enabled: event.target.checked })
                                    }
                                />
                                <span className="text-ui text-ink" aria-hidden="true">
                                    {draft.enabled ? words.on : words.off}
                                </span>
                            </div>
                            {admission !== null && (
                                <p className="text-small text-ink-3">
                                    {words.approved(admission.allowlist.approved)}
                                </p>
                            )}
                        </>
                    )}
                </SettingRow>
                <SettingRow
                    id="allowlistMessage"
                    field
                    changed={canManage && changed.includes('allowlistMessage')}
                >
                    {(ids) => (
                        <MessageField
                            ids={ids}
                            value={draft.message}
                            problem={tried ? messageProblem : null}
                            placeholders={PLACEHOLDERS}
                            example={words.example}
                            words={words}
                            onChange={(message) => setDraft({ ...draft, message })}
                        >
                            {!draft.message.includes('{id}') && messageProblem === null && (
                                <Notice tone="warning" live={false}>
                                    {words.idMissing}
                                </Notice>
                            )}
                        </MessageField>
                    )}
                </SettingRow>
            </SettingsGroup>
        </fieldset>
    );
}
