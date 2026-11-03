import { useMutation } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Plus, X } from 'lucide-react';
import { useState } from 'react';

import { saveArtifactSettings, settingsIssues } from '../api/artifacts';
import { ApiError } from '../api/client';
import type { ArtifactSettings } from '../api/generated/ArtifactSettings';
import type { ArtifactSettingsIssue } from '../api/generated/ArtifactSettingsIssue';
import type { ArtifactsStatus } from '../api/generated/ArtifactsStatus';
import { Permission } from '../api/generated/Permission';
import { useArtifacts } from '../artifacts/context';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { FieldError, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { Switch } from '../components/Switch';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import { cn } from '../lib/cn';
import { SettingsForm, useSection } from './Form';
import { CategoryHead, SettingRow, SettingsGroup } from './Row';

type Outcome = { tone: 'success' | 'danger'; message: string };

/** Which builds Observer lists, keeps and falls back to: what sat at the bottom of the Artifacts page. */
export function BuildSettings() {
    const { t } = useI18n();
    const link = useLinkState();
    const { status: live, refused } = useArtifacts();
    // While the link is down the category keeps the last status it had, so that what is typed stays.
    const [last, setLast] = useState(live);
    if (live !== null && live !== last) {
        setLast(live);
    }
    const status = live ?? (refused ? null : last);

    return (
        <>
            <CategoryHead
                category="builds"
                aside={
                    <Link
                        to="/artifacts"
                        className="inline-flex items-center gap-1.5 text-ui font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                    >
                        {t.settings.builds.openArtifacts}
                        <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                    </Link>
                }
            />
            {status !== null && live === null && <Notice tone="warning">{t.artifacts.outOfDate}</Notice>}
            {status === null ? (
                <section className="card">
                    <div className="card-body flex flex-col gap-3" aria-busy={link !== 'offline'}>
                        {link === 'live' || link === 'connecting' ? (
                            <>
                                <span className="sr-only">{t.artifacts.loading}</span>
                                <span className="skeleton w-1/3" />
                                <span className="skeleton w-1/2" />
                            </>
                        ) : (
                            <p className="text-ink-3">{t.artifacts.unknown}</p>
                        )}
                    </div>
                </section>
            ) : (
                <SettingsForm>
                    <BuildsForm status={status} />
                </SettingsForm>
            )}
        </>
    );
}

function BuildsForm({ status }: { status: ArtifactsStatus }) {
    const { t } = useI18n();
    const me = useMe();
    const words = t.artifacts.settings;
    const canManage = allows(me, Permission.ArtifactsManage);
    const [shown, setShown] = useState({ settings: status.settings, version: status.version });
    const [draft, setDraft] = useState<ArtifactSettings>(status.settings);
    const [branch, setBranch] = useState('');
    const [issues, setIssues] = useState<ArtifactSettingsIssue[]>([]);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const changed = (Object.keys(draft) as (keyof ArtifactSettings)[]).filter(
        (key) => JSON.stringify(draft[key]) !== JSON.stringify(shown.settings[key]),
    );
    const stale = status.version !== shown.version;
    if (stale && changed.length === 0) {
        setShown({ settings: status.settings, version: status.version });
        setDraft(status.settings);
    }
    const save = useMutation({
        mutationFn: () => saveArtifactSettings({ settings: draft, version: shown.version }),
        onMutate: () => {
            setOutcome(null);
            setIssues([]);
        },
        onSuccess: (saved) => {
            setShown({ settings: saved.settings, version: saved.version });
            setDraft(saved.settings);
            setOutcome({ tone: 'success', message: words.saved });
        },
        onError: (error) => {
            if (error instanceof ApiError && error.code === 'invalid_artifact_settings') {
                setIssues(settingsIssues(error.params));
            }
            setOutcome({ tone: 'danger', message: errorMessage(t, error) });
        },
    });

    useSection({
        changed: canManage ? changed : [],
        pending: save.isPending,
        save: () => {
            // A count of builds is whole: 2.5 is out of range here, as Observer would find it.
            if (!Number.isInteger(draft.keepBuilds) || draft.keepBuilds < 1 || draft.keepBuilds > 20) {
                setOutcome(null);
                setIssues([{ field: 'keepBuilds', index: null, problem: 'outOfRange' }]);
                return;
            }
            save.mutate();
        },
        discard: () => {
            setShown({ settings: status.settings, version: status.version });
            setDraft(status.settings);
            setIssues([]);
            setOutcome({ tone: 'success', message: words.discarded });
        },
    });

    const issueOf = (field: ArtifactSettingsIssue['field'], index: number | null = null) => {
        const found = issues.find((issue) => issue.field === field && issue.index === index);
        return found === undefined ? undefined : words.problems[found.problem];
    };
    const addBranch = () => {
        const name = branch.trim();
        if (name !== '' && !draft.branches.includes(name)) {
            setDraft({ ...draft, branches: [...draft.branches, name] });
            setBranch('');
        }
    };
    const is = (key: keyof ArtifactSettings) => canManage && changed.includes(key);

    return (
        <fieldset disabled={!canManage} className="flex min-w-0 flex-col gap-4">
            {!canManage && <Notice>{words.readOnly}</Notice>}
            {stale && changed.length > 0 && <Notice tone="warning">{words.changedElsewhere}</Notice>}
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            <SettingsGroup title={t.settings.categories.builds.name}>
                <SettingRow id="keepBuilds" field changed={is('keepBuilds')}>
                    {(ids) => (
                        <>
                            <input
                                id={ids.control}
                                type="number"
                                inputMode="numeric"
                                min={1}
                                max={20}
                                className={cn(inputClass, 'max-w-32')}
                                aria-invalid={issueOf('keepBuilds') ? true : undefined}
                                aria-describedby={
                                    issueOf('keepBuilds') ? `${ids.control}-error ${ids.about}` : ids.about
                                }
                                value={String(draft.keepBuilds)}
                                onChange={(event) =>
                                    setDraft({ ...draft, keepBuilds: Number(event.target.value) })
                                }
                            />
                            {issueOf('keepBuilds') && (
                                <FieldError id={`${ids.control}-error`}>{issueOf('keepBuilds')}</FieldError>
                            )}
                        </>
                    )}
                </SettingRow>
                <SettingRow id="autoRollback" field changed={is('autoRollback')}>
                    {(ids) => (
                        <Switch
                            id={ids.control}
                            aria-describedby={ids.about}
                            checked={draft.autoRollback}
                            onChange={(event) => setDraft({ ...draft, autoRollback: event.target.checked })}
                        />
                    )}
                </SettingRow>
                <SettingRow id="branches" changed={is('branches')}>
                    {(ids) => (
                        <fieldset aria-labelledby={ids.label} className="min-w-0 flex flex-col gap-2">
                            <ul className="flex flex-wrap gap-1.5">
                                {draft.branches.length === 0 && (
                                    <li className="text-small text-ink-3">{words.noBranch}</li>
                                )}
                                {draft.branches.map((name, index) => (
                                    <li key={name} className="flex flex-col gap-1">
                                        <span className="inline-flex h-7 items-center gap-1 rounded-pill border border-line bg-surface-2 pr-1 pl-2.5 font-mono text-small text-ink-2 max-md:h-11">
                                            {name}
                                            <button
                                                type="button"
                                                className="grid size-5 place-items-center rounded-pill hover:bg-hover focus-visible:outline-2 focus-visible:outline-accent max-md:size-11"
                                                aria-label={words.removeBranch(name)}
                                                onClick={() =>
                                                    setDraft({
                                                        ...draft,
                                                        branches: draft.branches.filter(
                                                            (_, at) => at !== index,
                                                        ),
                                                    })
                                                }
                                            >
                                                <X size={12} strokeWidth={2} aria-hidden="true" />
                                            </button>
                                        </span>
                                        {issueOf('branches', index) && (
                                            <FieldError>{issueOf('branches', index)}</FieldError>
                                        )}
                                    </li>
                                ))}
                            </ul>
                            {issueOf('branches') && <FieldError>{issueOf('branches')}</FieldError>}
                            <div className="flex max-w-md flex-wrap gap-2">
                                <input
                                    aria-label={words.branchName}
                                    className={cn(inputClass, 'min-w-0 flex-1 font-mono text-ui')}
                                    value={branch}
                                    spellCheck={false}
                                    autoCapitalize="none"
                                    onChange={(event) => setBranch(event.target.value)}
                                    onKeyDown={(event) => {
                                        if (event.key === 'Enter') {
                                            event.preventDefault();
                                            addBranch();
                                        }
                                    }}
                                />
                                <Button
                                    icon={Plus}
                                    onClick={addBranch}
                                    disabled={draft.branches.length >= 10}
                                >
                                    {words.addBranch}
                                </Button>
                            </div>
                        </fieldset>
                    )}
                </SettingRow>
            </SettingsGroup>
        </fieldset>
    );
}
