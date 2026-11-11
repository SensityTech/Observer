import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { Construction, Plus, Search } from 'lucide-react';
import { useId, useState, type KeyboardEvent } from 'react';

import { useAdmission } from '../admission/context';
import { MAX_NOTE } from '../api/allowlist';
import { characters, MAX_MESSAGE } from '../api/bans';
import type { MaintenancePass } from '../api/generated/MaintenancePass';
import type { MaintenanceSettings as Settings } from '../api/generated/MaintenanceSettings';
import { Permission } from '../api/generated/Permission';
import type { PlayerSummary } from '../api/generated/PlayerSummary';
import {
    addPass,
    maintenancePassesQuery,
    maintenanceSettingsQuery,
    removePass,
    saveMaintenanceSettings,
} from '../api/maintenance';
import { playersFoundQuery, shortLicence } from '../api/players';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Chip } from '../components/Chip';
import { CopyValue } from '../components/CopyValue';
import { FieldError, inputClass, TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { zoneDate } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useDebounced } from '../lib/useDebounced';
import { SettingsForm, useSection } from './Form';
import { MessageField } from './MessageField';
import { CategoryHead, SettingRow, SettingsGroup, type RowIds } from './Row';

type Outcome = { tone: 'success' | 'danger'; message: string };
const PLACEHOLDERS = ['name'] as const;
/** What FiveM's server list keeps of a server's name, as the API takes it. */
const MAX_SERVER_NAME = 40;
/** The settings as the form holds them: no name is an empty field. */
const drafted = (settings: Settings) => ({
    message: settings.message,
    serverName: settings.serverName ?? '',
});
const LICENCE = /^(license:)?[0-9a-f]{40}$/i;
/** How long typing pauses before the players a name finds are asked for. */
const PAUSE_MS = 200;
const SUGGESTED = 6;

/** What the list calls a player on it: their name, the owner's note, or their licence cut short. */
const called = (pass: MaintenancePass) => pass.name ?? pass.note ?? shortLicence(pass.license);

/** What a player refused during maintenance reads, who still gets in, and the server's name meanwhile. */
export function MaintenanceSettings() {
    const { t } = useI18n();
    const settings = useQuery(maintenanceSettingsQuery);
    return (
        <>
            <CategoryHead category="maintenance" />
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
                    {t.settings.maintenance.loadFailed} {errorMessage(t, settings.error)}
                </Notice>
            )}
            {settings.isSuccess && (
                <SettingsForm>
                    <MaintenanceForm stored={settings.data} />
                </SettingsForm>
            )}
        </>
    );
}

function MaintenanceForm({ stored }: { stored: Settings }) {
    const { t } = useI18n();
    const words = t.settings.maintenance;
    const me = useMe();
    const client = useQueryClient();
    const maintenance = useAdmission()?.maintenance;
    const canManage = allows(me, Permission.SettingsManage);
    const [draft, setDraft] = useState(() => drafted(stored));
    const [tried, setTried] = useState(false);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const serverName = draft.serverName.trim();
    const messageProblem =
        draft.message.trim() === ''
            ? words.messageMissing
            : characters(draft.message.trim()) > MAX_MESSAGE
              ? words.messageTooLong(MAX_MESSAGE)
              : null;
    const nameProblem = characters(serverName) > MAX_SERVER_NAME ? words.nameTooLong(MAX_SERVER_NAME) : null;
    const changed = [
        ...(draft.message === stored.message ? [] : (['maintenanceMessage'] as const)),
        ...(serverName === (stored.serverName ?? '') ? [] : (['maintenanceServerName'] as const)),
    ];
    const save = useMutation({
        mutationFn: () =>
            saveMaintenanceSettings({
                message: draft.message,
                serverName: serverName === '' ? null : serverName,
            }),
        onMutate: () => setOutcome(null),
        onSuccess: (saved) => {
            client.setQueryData(maintenanceSettingsQuery.queryKey, saved);
            setDraft(drafted(saved));
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
            if (messageProblem === null && nameProblem === null) {
                save.mutate();
            }
        },
        discard: () => {
            setDraft(drafted(stored));
            setTried(false);
            setOutcome({ tone: 'success', message: words.discarded });
        },
    });

    return (
        <fieldset disabled={!canManage} className="flex min-w-0 flex-col gap-4">
            {!canManage && <Notice>{words.readOnly}</Notice>}
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            <SettingsGroup
                title={t.settings.categories.maintenance.name}
                aside={
                    maintenance === undefined ? undefined : maintenance.on ? (
                        <Chip tone="warning" icon={Construction}>
                            {words.on}
                        </Chip>
                    ) : (
                        <Chip>{words.off}</Chip>
                    )
                }
            >
                <SettingRow
                    id="maintenanceMessage"
                    field
                    changed={canManage && changed.includes('maintenanceMessage')}
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
                        />
                    )}
                </SettingRow>
                <SettingRow id="maintenancePasses">
                    {(ids) => <Passes ids={ids} canManage={canManage} />}
                </SettingRow>
                <SettingRow
                    id="maintenanceServerName"
                    field
                    changed={canManage && changed.includes('maintenanceServerName')}
                >
                    {(ids) => (
                        <>
                            <input
                                id={ids.control}
                                className={inputClass}
                                aria-invalid={(tried && nameProblem !== null) || undefined}
                                aria-describedby={
                                    tried && nameProblem !== null
                                        ? `${ids.control}-error ${ids.about}`
                                        : ids.about
                                }
                                value={draft.serverName}
                                spellCheck={false}
                                autoComplete="off"
                                onChange={(event) => setDraft({ ...draft, serverName: event.target.value })}
                            />
                            {tried && nameProblem !== null && (
                                <FieldError id={`${ids.control}-error`}>{nameProblem}</FieldError>
                            )}
                        </>
                    )}
                </SettingRow>
            </SettingsGroup>
        </fieldset>
    );
}

/** The players maintenance lets through, each taken off in a click, and the field that adds one. */
function Passes({ ids, canManage }: { ids: RowIds; canManage: boolean }) {
    const { t } = useI18n();
    const words = t.settings.maintenance.passes;
    const me = useMe();
    const client = useQueryClient();
    const list = useQuery(maintenancePassesQuery);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const remove = useMutation({
        mutationFn: (pass: MaintenancePass) => removePass(pass.id),
        onMutate: () => setOutcome(null),
        onSuccess: (_, pass) => setOutcome({ tone: 'success', message: words.removedNotice(called(pass)) }),
        onError: (error) => setOutcome({ tone: 'danger', message: errorMessage(t, error) }),
        onSettled: () => client.invalidateQueries(maintenancePassesQuery),
    });
    const seesPlayers = allows(me, Permission.PlayersView);
    const zone = list.data?.zone ?? 'UTC';

    return (
        <>
            {list.isPending && (
                <div className="flex flex-col gap-2" aria-busy="true">
                    <span className="sr-only">{words.loading}</span>
                    <span className="skeleton w-2/3" />
                    <span className="skeleton w-1/2" />
                </div>
            )}
            {list.isError && (
                <Notice tone="danger">
                    {words.loadFailed} {errorMessage(t, list.error)}
                </Notice>
            )}
            {list.isSuccess &&
                (list.data.passes.length === 0 ? (
                    <p className="text-small text-ink-3">{words.empty}</p>
                ) : (
                    <ul aria-labelledby={ids.label} className="flex flex-col rounded-box border border-line">
                        {list.data.passes.map((pass) => (
                            <li
                                key={pass.id}
                                className="flex flex-wrap items-center gap-x-4 gap-y-1 border-b border-line px-3.5 py-2.5 last:border-b-0"
                            >
                                <div className="flex min-w-0 flex-[1_1_12rem] flex-col gap-0.5">
                                    <PassName pass={pass} link={seesPlayers} />
                                    <CopyValue
                                        value={pass.license}
                                        shown={shortLicence(pass.license)}
                                        label={t.players.copyLicence(pass.license)}
                                    />
                                </div>
                                <span className="text-small text-ink-3">
                                    {words.added(pass.by, zoneDate(pass.addedAt, zone))}
                                </span>
                                {canManage && (
                                    <Button
                                        tone="ghost"
                                        size="sm"
                                        className="ml-auto"
                                        aria-label={words.removeOf(called(pass))}
                                        pending={remove.isPending && remove.variables.id === pass.id}
                                        onClick={() => remove.mutate(pass)}
                                    >
                                        {words.remove}
                                    </Button>
                                )}
                            </li>
                        ))}
                    </ul>
                ))}
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            {canManage && (
                <AddPass
                    searches={seesPlayers}
                    onAdded={(who) => setOutcome({ tone: 'success', message: words.addedNotice(who) })}
                    onStart={() => setOutcome(null)}
                />
            )}
        </>
    );
}

function PassName({ pass, link }: { pass: MaintenancePass; link: boolean }) {
    const { t } = useI18n();
    if (pass.name === null) {
        return (
            <span className={cn('[overflow-wrap:anywhere]', pass.note === null && 'text-ink-2 italic')}>
                {pass.note ?? t.settings.maintenance.passes.noName}
            </span>
        );
    }
    return (
        <>
            {pass.player !== null && link ? (
                <Link
                    to="/players/$id"
                    params={{ id: String(pass.player) }}
                    className="font-medium text-ink underline-offset-2 [overflow-wrap:anywhere] hover:underline"
                >
                    {pass.name}
                </Link>
            ) : (
                <span className="font-medium text-ink [overflow-wrap:anywhere]">{pass.name}</span>
            )}
            {pass.note !== null && (
                <span className="text-meta text-ink-3 [overflow-wrap:anywhere]">{pass.note}</span>
            )}
        </>
    );
}

/**
 * Adds a player to the list: a combobox that lists the players a typed name finds, for an account
 * that sees players, or takes a licence pasted whole, with a note that says whose it is.
 */
function AddPass({
    searches,
    onAdded,
    onStart,
}: {
    searches: boolean;
    onAdded: (who: string) => void;
    onStart: () => void;
}) {
    const { t } = useI18n();
    const words = t.settings.maintenance.passes;
    const client = useQueryClient();
    const id = useId();
    const [text, setText] = useState('');
    const [picked, setPicked] = useState<PlayerSummary | null>(null);
    const [note, setNote] = useState('');
    const [open, setOpen] = useState(false);
    const [active, setActive] = useState(-1);
    const [tried, setTried] = useState(false);
    const typed = text.trim();
    const asked = useDebounced(typed, PAUSE_MS);
    const licence = LICENCE.test(typed);
    const searching = searches && picked === null && !licence && asked.length >= 2;
    const found = useQuery({
        ...playersFoundQuery(asked),
        enabled: searching,
        placeholderData: keepPreviousData,
    });
    const options = searching
        ? (found.data?.players ?? [])
              .toSorted((first, second) => Number(second.online) - Number(first.online))
              .slice(0, SUGGESTED)
        : [];
    const answered = searching && found.isSuccess && !found.isPlaceholderData && asked === typed;
    const expanded = open && searching && (options.length > 0 || answered);
    const current = active < options.length ? active : -1;
    const noteProblem = characters(note.trim()) > MAX_NOTE ? words.noteTooLong(MAX_NOTE) : null;
    const problem = picked === null && !licence ? words.pickFirst : null;

    const add = useMutation({
        mutationFn: () => (picked === null ? addPass(typed, note) : addPass(picked.license, '')),
        onMutate: onStart,
        onSuccess: async (pass) => {
            await client.invalidateQueries(maintenancePassesQuery);
            setText('');
            setPicked(null);
            setNote('');
            setTried(false);
            onAdded(called(pass));
        },
    });
    const submit = () => {
        setTried(true);
        setOpen(false);
        if (problem === null && noteProblem === null && !add.isPending) {
            add.mutate();
        }
    };
    const pick = (player: PlayerSummary) => {
        setPicked(player);
        setText(player.name);
        setOpen(false);
        setActive(-1);
        setTried(false);
    };
    const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault();
            if (!expanded) {
                setOpen(true);
            } else if (options.length > 0) {
                const step = event.key === 'ArrowDown' ? 1 : -1;
                const from = current === -1 && step === -1 ? 0 : current;
                setActive((from + step + options.length) % options.length);
            }
        } else if (event.key === 'Enter') {
            // The row lives in the category's form, whose own Enter saves the settings.
            event.preventDefault();
            const option = expanded ? options[current] : undefined;
            if (option === undefined) {
                submit();
            } else {
                pick(option);
            }
        } else if (event.key === 'Escape' && expanded) {
            event.preventDefault();
            setOpen(false);
        }
    };
    const field = searches ? words.field : words.fieldLicence;
    const shownProblem = tried ? problem : null;

    return (
        <div className="flex flex-col gap-2">
            <div className="flex items-start gap-2 max-md:flex-col max-md:items-stretch">
                <div className="relative min-w-0 flex-1">
                    <Search
                        size={16}
                        strokeWidth={1.5}
                        className="pointer-events-none absolute top-1/2 left-2.75 -translate-y-1/2 text-ink-3"
                        aria-hidden="true"
                    />
                    <input
                        // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role -- an ARIA combobox: a select cannot take a pasted licence
                        role="combobox"
                        aria-label={field}
                        aria-expanded={expanded}
                        aria-controls={`${id}-players`}
                        aria-autocomplete="list"
                        aria-activedescendant={current === -1 ? undefined : `${id}-player-${current}`}
                        aria-invalid={shownProblem !== null || undefined}
                        aria-describedby={shownProblem === null ? `${id}-hint` : `${id}-error`}
                        className={cn(inputClass, 'pl-8.5')}
                        placeholder={field}
                        value={text}
                        spellCheck={false}
                        autoCapitalize="none"
                        autoComplete="off"
                        onChange={(event) => {
                            setText(event.target.value);
                            setPicked(null);
                            setOpen(true);
                            setActive(-1);
                            setTried(false);
                        }}
                        onKeyDown={onKeyDown}
                        onBlur={() => setOpen(false)}
                    />
                </div>
                <Button icon={Plus} pending={add.isPending} onClick={submit}>
                    {words.add}
                </Button>
            </div>
            {/* In the flow, under the field: the card it sits in cuts off whatever hangs out of it. */}
            <div
                className={cn(
                    'rounded-box border border-line-strong bg-surface p-1.5',
                    !expanded && 'hidden',
                )}
            >
                <div
                    // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role -- the combobox's own listbox, which the field drives
                    role="listbox"
                    id={`${id}-players`}
                    aria-label={words.suggestions}
                >
                    {options.map((player, index) => (
                        // oxlint-disable-next-line jsx-a11y/click-events-have-key-events -- the field's keys choose it
                        <div
                            key={player.id}
                            id={`${id}-player-${index}`}
                            // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role -- an option of the combobox's listbox
                            role="option"
                            tabIndex={-1}
                            aria-selected={index === current}
                            className={cn(
                                'flex cursor-pointer flex-wrap items-center gap-x-2.5 gap-y-0.5 rounded-sm px-2.5 py-2 max-md:min-h-11',
                                index === current && 'bg-accent-soft',
                            )}
                            // The field keeps the keyboard: a press on an option must not blur it first.
                            onMouseDown={(event) => event.preventDefault()}
                            onMouseMove={() => setActive(index)}
                            onClick={() => pick(player)}
                        >
                            <span className="min-w-0 font-medium text-ink [overflow-wrap:anywhere]">
                                {player.name}
                            </span>
                            <span className="font-mono text-small text-ink-3">
                                {shortLicence(player.license)}
                            </span>
                            {player.online && (
                                <span className="ml-auto inline-flex items-center gap-1.5 text-small text-ink-2">
                                    <span className="led led-success" aria-hidden="true" />
                                    {words.online}
                                </span>
                            )}
                        </div>
                    ))}
                </div>
                {options.length === 0 && <p className="px-2.5 py-2 text-small text-ink-3">{words.noMatch}</p>}
            </div>
            {shownProblem !== null && <FieldError id={`${id}-error`}>{shownProblem}</FieldError>}
            {picked !== null && (
                <p className="text-small text-ink-2">
                    <output>{words.picked(picked.name, shortLicence(picked.license))}</output>
                </p>
            )}
            {licence && picked === null && (
                <TextField
                    label={
                        <>
                            {words.note} <span className="font-normal text-ink-3">· {t.common.optional}</span>
                        </>
                    }
                    hint={words.noteHint}
                    error={tried && noteProblem !== null ? noteProblem : undefined}
                    value={note}
                    autoComplete="off"
                    onChange={(event) => setNote(event.target.value)}
                    onKeyDown={(event) => {
                        if (event.key === 'Enter') {
                            event.preventDefault();
                            submit();
                        }
                    }}
                />
            )}
            {add.isError && <Notice tone="danger">{errorMessage(t, add.error)}</Notice>}
            <p id={`${id}-hint`} className="text-small text-ink-3">
                {searches ? words.hint : words.hintLicence}
            </p>
        </div>
    );
}
