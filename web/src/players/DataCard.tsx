import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { FileInput, Settings2 } from 'lucide-react';
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
import { Card } from '../components/Card';
import { Checkbox } from '../components/Checkbox';
import { TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { PathField } from '../components/PathField';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { useVisit } from './words';

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
export function DataCard({ zone }: { zone: string }) {
    const { t } = useI18n();
    const words = t.players.data;
    const settings = useQuery(playersSettingsQuery);
    return (
        <Card id="players-data" icon={Settings2} title={words.title} meta={words.meta}>
            <div className="flex flex-col gap-8">
                <section className="flex max-w-2xl flex-col gap-4" aria-labelledby="players-retention">
                    <div className="flex flex-col gap-1">
                        <h3 id="players-retention" className="text-body font-semibold text-ink">
                            {words.retention}
                        </h3>
                        <p className="text-small text-ink-3">{words.retentionLead}</p>
                    </div>
                    {settings.isPending && (
                        <div className="flex flex-col gap-3" aria-busy="true">
                            <span className="sr-only">{words.loading}</span>
                            <span className="skeleton w-1/2" />
                            <span className="skeleton w-1/3" />
                        </div>
                    )}
                    {settings.isError && (
                        <Notice tone="danger">
                            {words.failed} {errorMessage(t, settings.error)}
                        </Notice>
                    )}
                    {settings.data !== undefined && <RetentionForm saved={settings.data} />}
                </section>
                <section
                    className="flex max-w-2xl flex-col gap-4 border-t border-line pt-6"
                    aria-labelledby="players-import"
                >
                    <div className="flex flex-col gap-1">
                        <h3 id="players-import" className="text-body font-semibold text-ink">
                            {words.import}
                        </h3>
                        <p className="text-small text-ink-3">{words.importLead}</p>
                    </div>
                    <ImportForm zone={zone} />
                </section>
            </div>
        </Card>
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
    const changes = (Object.keys(draft) as (keyof Draft)[]).filter((key) => draft[key] !== kept[key]).length;
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
    const field = (key: DayField) => (
        <TextField
            label={words[key]}
            hint={words.ranges[key]}
            error={issues.includes(key) ? words.outOfRange(words.ranges[key]) : undefined}
            type="number"
            inputMode="numeric"
            min={FEWEST_DAYS[key]}
            className="max-w-md"
            value={draft[key]}
            onChange={(event) => setDraft({ ...draft, [key]: event.target.value })}
        />
    );
    return (
        <form
            className="flex flex-col gap-5"
            noValidate
            onSubmit={(event) => {
                event.preventDefault();
                setChecked(true);
                if (retentionIssues(settingsOf(draft)).length === 0 && !save.isPending) {
                    save.mutate();
                }
            }}
        >
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            {field('linkDays')}
            {field('addressDays')}
            {field('dayDays')}
            <div className="flex flex-col gap-3">
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
                    <TextField
                        label={words.profileDays}
                        hint={words.ranges.profileDays}
                        error={
                            issues.includes('profileDays')
                                ? words.outOfRange(words.ranges.profileDays)
                                : undefined
                        }
                        type="number"
                        inputMode="numeric"
                        min={FEWEST_DAYS.profileDays}
                        className="max-w-md"
                        value={draft.profileDays}
                        onChange={(event) => setDraft({ ...draft, profileDays: event.target.value })}
                    />
                )}
            </div>
            {changes > 0 && (
                <div className="sticky bottom-3 z-10 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-box border border-line-strong bg-surface px-3.5 py-2.5 shadow-pop">
                    <output className="mr-auto text-ui text-ink-2">{words.changes(changes)}</output>
                    <Button
                        tone="ghost"
                        className="max-md:flex-1"
                        onClick={() => {
                            setDraft(draftOf(saved));
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

function ImportForm({ zone }: { zone: string }) {
    const { t } = useI18n();
    const words = t.players.data;
    const client = useQueryClient();
    const imports = useQuery(importQuery);
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
    const writing = imports.data?.state === 'writing';
    return (
        <form
            className="flex flex-col gap-4"
            noValidate
            onSubmit={(event) => {
                event.preventDefault();
                setTouched(true);
                if (path.trim() !== '' && !start.isPending) {
                    start.mutate();
                }
            }}
        >
            <PathField
                label={words.path}
                hint={words.pathHint}
                value={path}
                error={touched && path.trim() === '' ? t.validation.required : (refused ?? undefined)}
                onChange={(value) => {
                    setPath(value);
                    start.reset();
                }}
            />
            {start.isError && refused === null && (
                <Notice tone="danger">
                    {start.error instanceof ApiError && start.error.code === 'job_running'
                        ? words.running
                        : errorMessage(t, start.error)}
                </Notice>
            )}
            <div>
                <Button type="submit" icon={FileInput} pending={start.isPending} disabled={writing}>
                    {words.start}
                </Button>
            </div>
            {imports.data != null && <ImportState state={imports.data} zone={zone} />}
        </form>
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
