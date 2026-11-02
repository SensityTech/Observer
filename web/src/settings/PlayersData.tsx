import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { FileInput } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';

import { ApiError } from '../api/client';
import type { PlayersImport } from '../api/generated/PlayersImport';
import type { PlayersSettings } from '../api/generated/PlayersSettings';
import {
    FEWEST_DAYS,
    allPlayers,
    importQuery,
    playersSettingsQuery,
    retentionIssues,
    savePlayersSettings,
    startImport,
} from '../api/players';
import { Button } from '../components/Button';
import { Checkbox } from '../components/Checkbox';
import { FieldError, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useVisit } from '../players/words';
import { useSchedule } from '../schedule/context';
import { useSection } from './Form';
import { GroupNotice, SettingRow, SettingsGroup } from './Row';

type Outcome = { tone: 'success' | 'danger'; message: string };

/** The profile retention offered when the owner stops keeping profiles for good: the default. */
const PROFILE_DAYS = 730;

type DayField = 'linkDays' | 'addressDays' | 'dayDays';

/** The retentions as typed: a field emptied stays empty rather than turning into 0. */
type Draft = Record<DayField, string> & { profileDays: string | null };

const draftOf = (settings: PlayersSettings): Draft => ({
    linkDays: String(settings.linkDays),
    addressDays: String(settings.addressDays),
    dayDays: String(settings.dayDays),
    profileDays: settings.profileDays === null ? null : String(settings.profileDays),
});

/** A whole number of days as typed; anything else, an empty field included, is out of every range. */
const days = (text: string) => (/^\d+$/.test(text.trim()) ? Number(text.trim()) : Number.NaN);

const settingsOf = (draft: Draft): PlayersSettings => ({
    linkDays: days(draft.linkDays),
    addressDays: days(draft.addressDays),
    dayDays: days(draft.dayDays),
    profileDays: draft.profileDays === null ? null : days(draft.profileDays),
});

/** The owner's: how long player data is kept, and txAdmin's players taken in. */
export function PlayersData() {
    const { t } = useI18n();
    const words = t.players.data;
    const settings = useQuery(playersSettingsQuery);
    return (
        <SettingsGroup title={t.settings.data.players}>
            {settings.isPending && (
                <GroupNotice>
                    <div className="flex flex-col gap-3 pb-4" aria-busy="true">
                        <span className="sr-only">{words.loading}</span>
                        <span className="skeleton w-1/2" />
                        <span className="skeleton w-1/3" />
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
            <SettingRow id="playersImport">{() => <PlayersImportForm />}</SettingRow>
        </SettingsGroup>
    );
}

function RetentionForm({ saved }: { saved: PlayersSettings }) {
    const { t } = useI18n();
    const words = t.players.data;
    const client = useQueryClient();
    const keepId = useId();
    const [draft, setDraft] = useState(() => draftOf(saved));
    const [checked, setChecked] = useState(false);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const issues = checked ? retentionIssues(settingsOf(draft)) : [];
    const kept = draftOf(saved);
    const changed = (Object.keys(draft) as (keyof Draft)[]).filter((key) => draft[key] !== kept[key]);
    const save = useMutation({
        mutationFn: () => savePlayersSettings(settingsOf(draft)),
        onMutate: () => setOutcome(null),
        onSuccess: (stored) => {
            client.setQueryData(playersSettingsQuery.queryKey, stored);
            setDraft(draftOf(stored));
            setChecked(false);
            setOutcome({ tone: 'success', message: words.saved });
        },
        onError: (error) => setOutcome({ tone: 'danger', message: errorMessage(t, error) }),
    });

    useSection({
        changed,
        pending: save.isPending,
        save: () => {
            setChecked(true);
            if (retentionIssues(settingsOf(draft)).length === 0) {
                save.mutate();
            }
        },
        discard: () => {
            setDraft(draftOf(saved));
            setChecked(false);
            setOutcome({ tone: 'success', message: words.discarded });
        },
    });

    const field = (key: DayField) => (
        <SettingRow id={key} field changed={changed.includes(key)}>
            {(ids) => (
                <>
                    <input
                        id={ids.control}
                        type="number"
                        inputMode="numeric"
                        min={FEWEST_DAYS[key]}
                        className={cn(inputClass, 'max-w-40')}
                        aria-invalid={issues.includes(key) || undefined}
                        aria-describedby={
                            issues.includes(key) ? `${ids.control}-error ${ids.about}` : ids.about
                        }
                        value={draft[key]}
                        onChange={(event) => setDraft({ ...draft, [key]: event.target.value })}
                    />
                    {issues.includes(key) && (
                        <FieldError id={`${ids.control}-error`}>
                            {words.outOfRange(words.ranges[key])}
                        </FieldError>
                    )}
                </>
            )}
        </SettingRow>
    );

    return (
        <>
            {outcome !== null && (
                <GroupNotice>
                    <Notice tone={outcome.tone}>{outcome.message}</Notice>
                </GroupNotice>
            )}
            {field('linkDays')}
            {field('addressDays')}
            {field('dayDays')}
            <SettingRow id="profileDays" changed={changed.includes('profileDays')}>
                {(ids) => (
                    <fieldset aria-labelledby={ids.label} className="min-w-0 flex flex-col gap-3">
                        <label
                            htmlFor={keepId}
                            className="-mx-2 flex cursor-pointer items-start gap-3 rounded-sm px-2 py-2 hover:bg-hover"
                        >
                            <Checkbox
                                id={keepId}
                                checked={draft.profileDays === null}
                                onChange={(event) =>
                                    setDraft({
                                        ...draft,
                                        profileDays: event.target.checked
                                            ? null
                                            : String(saved.profileDays ?? PROFILE_DAYS),
                                    })
                                }
                            />
                            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                                <span className="text-body font-medium text-ink">{words.keepForGood}</span>
                                <span className="text-small text-ink-2">{words.keepForGoodHint}</span>
                            </span>
                        </label>
                        {draft.profileDays !== null && (
                            <>
                                <input
                                    type="number"
                                    inputMode="numeric"
                                    min={FEWEST_DAYS.profileDays}
                                    aria-label={words.profileDays}
                                    className={cn(inputClass, 'max-w-40')}
                                    aria-invalid={issues.includes('profileDays') || undefined}
                                    aria-describedby={
                                        issues.includes('profileDays')
                                            ? `${ids.control}-error ${ids.about}`
                                            : ids.about
                                    }
                                    value={draft.profileDays}
                                    onChange={(event) =>
                                        setDraft({ ...draft, profileDays: event.target.value })
                                    }
                                />
                                {issues.includes('profileDays') && (
                                    <FieldError id={`${ids.control}-error`}>
                                        {words.outOfRange(words.ranges.profileDays)}
                                    </FieldError>
                                )}
                            </>
                        )}
                    </fieldset>
                )}
            </SettingRow>
        </>
    );
}

/** txAdmin's players database, read on the host: an action, saved by its own button rather than the bar. */
function PlayersImportForm() {
    const { t } = useI18n();
    const words = t.players.data;
    const client = useQueryClient();
    const imports = useQuery(importQuery);
    const id = useId();
    const zone = useSchedule().zone ?? 'UTC';
    const [path, setPath] = useState('');
    const [touched, setTouched] = useState(false);
    const start = useMutation({
        mutationFn: () => startImport(path.trim()),
        onSuccess: async (started) => {
            // A read still on its way would put the status from before the start back, and end the polling.
            await client.cancelQueries({ queryKey: importQuery.queryKey });
            client.setQueryData(importQuery.queryKey, started);
        },
    });
    // Once written, the players it brought change the list, the community figures and the profiles.
    const state = imports.data?.state;
    const before = useRef(state);
    useEffect(() => {
        if (before.current === 'writing' && state !== undefined && state !== 'writing') {
            void client.invalidateQueries(allPlayers);
        }
        before.current = state;
    }, [state, client]);
    const refused =
        start.error instanceof ApiError && start.error.code === 'invalid_players_file'
            ? errorMessage(t, start.error)
            : null;
    const missing = touched && path.trim() === '';
    const problem = missing ? t.validation.required : refused;
    const writing = imports.data?.state === 'writing';
    const go = () => {
        setTouched(true);
        if (path.trim() !== '' && !start.isPending) {
            start.mutate();
        }
    };

    return (
        <div className="flex flex-col gap-3">
            <div className="flex min-w-0 flex-col gap-1.5">
                <label htmlFor={id} className="text-ui font-medium text-ink">
                    {words.path}
                </label>
                <input
                    id={id}
                    className={cn(inputClass, 'font-mono text-ui')}
                    spellCheck={false}
                    autoCapitalize="none"
                    autoComplete="off"
                    aria-invalid={problem !== null || undefined}
                    aria-describedby={problem === null ? `${id}-hint` : `${id}-error ${id}-hint`}
                    value={path}
                    onChange={(event) => {
                        setPath(event.target.value);
                        start.reset();
                    }}
                    // Enter imports, rather than saving the category's settings.
                    onKeyDown={(event) => {
                        if (event.key === 'Enter') {
                            event.preventDefault();
                            go();
                        }
                    }}
                />
                {problem !== null && <FieldError id={`${id}-error`}>{problem}</FieldError>}
                <p id={`${id}-hint`} className="text-small text-ink-3">
                    {words.pathHint}
                </p>
            </div>
            {start.isError && refused === null && (
                <Notice tone="danger">
                    {start.error instanceof ApiError && start.error.code === 'job_running'
                        ? words.running
                        : errorMessage(t, start.error)}
                </Notice>
            )}
            <div>
                <Button icon={FileInput} pending={start.isPending} disabled={writing} onClick={go}>
                    {words.start}
                </Button>
            </div>
            {imports.data != null && <ImportState state={imports.data} zone={zone} />}
        </div>
    );
}

/** The import under way, with how far it is, or how the last one ended. */
function ImportState({ state, zone }: { state: PlayersImport; zone: string }) {
    const { t } = useI18n();
    const words = t.players.data;
    const visit = useVisit(zone);
    const at = visit(state.finishedAt ?? state.startedAt);
    if (state.state === 'writing') {
        const progress = words.writing(state.written, state.read);
        return (
            <div className="flex flex-col gap-2">
                <p className="text-small text-ink-2">{words.started}</p>
                <progress
                    aria-label={words.progress}
                    aria-valuetext={progress}
                    max={Math.max(state.read, 1)}
                    value={state.written}
                    className="h-1.5 w-full appearance-none overflow-hidden rounded-pill bg-surface-2 [&::-moz-progress-bar]:bg-accent [&::-webkit-progress-bar]:bg-surface-2 [&::-webkit-progress-value]:rounded-pill [&::-webkit-progress-value]:bg-accent"
                />
                <output className="block text-small text-ink-2">{progress}</output>
            </div>
        );
    }
    return state.state === 'finished' ? (
        <Notice tone="success">
            {words.finished(at, state.imported, state.updated, state.forgotten, state.invalid)}
        </Notice>
    ) : (
        <Notice tone="warning">{words.stopped(at, state.written, state.read)}</Notice>
    );
}
