import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, useLocation } from '@tanstack/react-router';
import { ArrowRight, ChevronDown, FolderSearch, Plus, X } from 'lucide-react';
import { useEffect, useId, useRef, useState, type Ref } from 'react';

import { ApiError } from '../api/client';
import type { LaunchField } from '../api/generated/LaunchField';
import type { LaunchIssue } from '../api/generated/LaunchIssue';
import type { LaunchSettings as Settings } from '../api/generated/LaunchSettings';
import type { OneSync } from '../api/generated/OneSync';
import type { StoredLaunch } from '../api/generated/StoredLaunch';
import {
    changedSettings,
    cleaned,
    DEFAULT_LAUNCH,
    HANG_SETTINGS,
    launchIssues,
    launchQuery,
    saveLaunch,
} from '../api/launch';
import { useArtifacts } from '../artifacts/context';
import { Button, buttonVariants } from '../components/Button';
import { FieldError, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { Switch } from '../components/Switch';
import type { Messages } from '../i18n/en';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { anchorOf, type SettingId } from './catalog';
import { keyboardLeft, SettingsForm, useSection } from './Form';
import { CategoryHead, SettingRow, SettingsGroup, type RowIds } from './Row';

/** An extra argument as the form edits it: the id keeps a row's fields when one above is removed. */
type Row = { id: number; name: string; value: string };

type Outcome = { tone: 'neutral' | 'success' | 'warning' | 'danger'; message: string };

const ONESYNC_MODES: OneSync[] = ['on', 'legacy', 'off'];

/** The setting each stored field is shown as. */
const SETTING_OF = {
    dataPath: 'dataPath',
    cfgFile: 'cfgFile',
    onesync: 'onesync',
    autoStart: 'autoStart',
    txadminEvents: 'txadminEvents',
    extraArguments: 'extraArguments',
    hangDetection: 'hangDetection',
    hangTimeoutSeconds: 'hangTimeout',
} satisfies Record<keyof Settings, SettingId>;

let lastRow = 0;
const toRows = (settings: Settings): Row[] =>
    settings.extraArguments.map((argument) => ({ id: ++lastRow, ...argument }));

/**
 * The hang timeout as the form edits it, in minutes. Observer takes whole minutes only, but a value
 * stored otherwise reads 8.33 rather than 8.333333333333334.
 */
const minutesOf = (settings: Settings) => String(Math.round((settings.hangTimeoutSeconds / 60) * 100) / 100);
/** The minutes typed as they are sent, in seconds: rounded, as 0.1 × 60 makes 6.000000000000001. */
const secondsOf = (minutes: string) =>
    minutes.trim() === '' ? Number.NaN : Math.round(Number(minutes) * 60);
/** A whole number of minutes from 1 to 60, as Observer takes it in seconds. */
const minutesFit = (minutes: string) => {
    const value = Number(minutes);
    return minutes.trim() !== '' && Number.isInteger(value) && value >= 1 && value <= 60;
};

/** What a save says: hang detection reaches a running server at once, the rest its next start. */
const savedMessage = (t: Messages, before: Settings, after: Settings) => {
    const changed = changedSettings(before, after);
    const hang = changed.some((key) => HANG_SETTINGS.has(key));
    if (!hang) {
        return t.server.saved;
    }
    return changed.every((key) => HANG_SETTINGS.has(key)) ? t.server.savedHang : t.server.savedBoth;
};

/** How Observer starts FXServer, the owner's: what was the Server page. */
export function LaunchSettings() {
    const { t } = useI18n();
    const launch = useQuery(launchQuery);

    return (
        <>
            <CategoryHead
                category="launch"
                aside={
                    <Link to="/server/setup" className={buttonVariants({ size: 'sm' })}>
                        <FolderSearch size={14} strokeWidth={1.5} aria-hidden="true" />
                        {t.settings.launch.runSetup}
                    </Link>
                }
            />
            {launch.isPending && (
                <div className="card flex flex-col gap-3 p-5">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-3/4" />
                    <span className="skeleton w-2/3" />
                    <span className="skeleton w-1/2" />
                </div>
            )}
            {launch.isError && (
                <Notice tone="danger">
                    {t.server.loadFailed} {errorMessage(t, launch.error)}
                </Notice>
            )}
            {/* A failed refetch keeps the data it had: the form, and what was typed in it, stay. */}
            {launch.data !== undefined && (
                <SettingsForm>
                    <LaunchForm saved={launch.data} />
                </SettingsForm>
            )}
        </>
    );
}

function LaunchForm({ saved }: { saved: StoredLaunch | null }) {
    const { t } = useI18n();
    const words = t.settings.launch;
    const queryClient = useQueryClient();
    // What the draft was started from: the settings as stored then, and the version a save names.
    const [shown, setShown] = useState(saved);
    const baseline = shown?.settings ?? DEFAULT_LAUNCH;
    const [fields, setFields] = useState<Settings>(baseline);
    const [rows, setRows] = useState<Row[]>(() => toRows(baseline));
    const [minutes, setMinutes] = useState(() => minutesOf(baseline));
    const [issues, setIssues] = useState<LaunchIssue[]>([]);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const notice = useRef<HTMLDivElement>(null);
    const nameInputs = useRef(new Map<number, HTMLInputElement>());
    const addButton = useRef<HTMLButtonElement>(null);
    /** Where the keyboard goes once a removed row is gone: another row's name, or the add button. */
    const focusAfterRemoval = useRef<number | 'add' | null>(null);
    const form = useRef<HTMLDivElement>(null);
    const showFirstIssue = useRef(false);
    const argumentsId = useId();
    // A search for the extra arguments, or a problem with one, opens them.
    const sought = useLocation({ select: (location) => location.hash === anchorOf('extraArguments') });
    const [advanced, setAdvanced] = useState(sought);
    const [soughtBefore, setSoughtBefore] = useState(sought);
    if (sought !== soughtBefore) {
        setSoughtBefore(sought);
        if (sought) {
            setAdvanced(true);
        }
    }

    const draft: Settings = {
        ...fields,
        extraArguments: rows.map(({ name, value }) => ({ name, value })),
        // Compared in the minutes the field shows: left as it was, it gives back the seconds stored,
        // so one the field cannot show exactly is never an unsaved change nobody made.
        hangTimeoutSeconds:
            minutes === minutesOf(baseline) ? baseline.hangTimeoutSeconds : secondsOf(minutes),
    };
    const changed = changedSettings(baseline, draft);
    const stale = (saved?.version ?? null) !== (shown?.version ?? null);

    const startFrom = (stored: StoredLaunch | null) => {
        const settings = stored?.settings ?? DEFAULT_LAUNCH;
        setShown(stored);
        setFields(settings);
        setMinutes(minutesOf(settings));
        // Rows keep their ids by position, so their inputs, and the keyboard in one, stay put.
        setRows((current) =>
            settings.extraArguments.map((argument, index) => ({
                id: current[index]?.id ?? ++lastRow,
                ...argument,
            })),
        );
    };

    // Another session saved since: with nothing typed yet, show what is stored now rather than
    // what it replaced, which a save would quietly bring back.
    if (stale && changed.length === 0) {
        startFrom(saved);
    }

    // What the save started from goes with it: by the time it answers, the form may show another.
    const save = useMutation({
        mutationFn: ({ settings }: { settings: Settings; before: Settings }) =>
            saveLaunch({ settings, version: shown?.version ?? null }),
        onSuccess: (stored, { settings, before }) => {
            queryClient.setQueryData(launchQuery.queryKey, stored);
            startFrom(stored);
            setIssues([]);
            setOutcome({ tone: 'success', message: savedMessage(t, before, settings) });
        },
        onError: (error) => {
            const conflict = error instanceof ApiError && error.code === 'conflict';
            if (conflict) {
                void queryClient.invalidateQueries({ queryKey: launchQuery.queryKey });
            }
            const refused = error instanceof ApiError && error.code === 'invalid_launch_settings';
            showFirstIssue.current = refused;
            const found = refused ? launchIssues(error.params) : [];
            if (found.some(({ field }) => field.startsWith('argument') || field === 'extraArguments')) {
                setAdvanced(true);
            }
            setIssues(found);
            setOutcome({
                tone: conflict ? 'warning' : 'danger',
                message: conflict ? t.server.changedElsewhere : errorMessage(t, error),
            });
        },
    });

    const issue = (field: LaunchField, index: number | null = null) => {
        const found = issues.find((each) => each.field === field && each.index === index);
        return found && t.server.problems[found.problem];
    };
    const forget = (field: LaunchField, index: number | null = null) =>
        setIssues((current) => current.filter((each) => each.field !== field || each.index !== index));

    const edit = <K extends keyof Settings>(key: K, value: Settings[K]) => {
        setFields((current) => ({ ...current, [key]: value }));
        setOutcome(null);
    };
    const editRow = (id: number, change: Partial<Row>) => {
        setRows((current) => current.map((row) => (row.id === id ? { ...row, ...change } : row)));
        setOutcome(null);
    };
    // Positions shift when a row goes: what was said about them no longer points at the right row.
    const forgetArguments = () =>
        setIssues((current) =>
            current.filter((each) => each.index === null && each.field !== 'extraArguments'),
        );

    useSection({
        changed: changed.map((key) => SETTING_OF[key]),
        pending: save.isPending,
        unsaved: saved === null,
        save: () => {
            setOutcome(null);
            // A fraction or an emptied field would be refused as a malformed request rather than
            // next to its field: it is checked before sending.
            if (!minutesFit(minutes)) {
                showFirstIssue.current = true;
                setIssues((current) => [
                    ...current.filter((each) => each.field !== 'hangTimeoutSeconds'),
                    { field: 'hangTimeoutSeconds', index: null, problem: 'outOfRange' },
                ]);
                return;
            }
            save.mutate({ settings: cleaned(draft), before: baseline });
        },
        discard: () => {
            startFrom(saved);
            setIssues([]);
            setOutcome({ tone: 'neutral', message: t.server.discarded });
        },
    });

    // Saving or discarding takes the save bar away, and with it the button the keyboard was on:
    // it moves to the notice saying what happened rather than back to the top of the document.
    useEffect(() => {
        if (outcome !== null && keyboardLeft()) {
            notice.current?.focus();
        }
    }, [outcome]);

    // The first field to fix may be far up, or under the save bar: it comes to the middle of the
    // screen, the keyboard in it.
    useEffect(() => {
        if (!showFirstIssue.current) {
            return;
        }
        showFirstIssue.current = false;
        const first = form.current?.querySelector<HTMLInputElement>('input[aria-invalid="true"]');
        first?.focus({ preventScroll: true });
        first?.scrollIntoView({ block: 'center' });
    });

    useEffect(() => {
        const target = focusAfterRemoval.current;
        if (target === null) {
            return;
        }
        focusAfterRemoval.current = null;
        (target === 'add' ? addButton.current : nameInputs.current.get(target))?.focus();
    });

    const is = (key: keyof Settings) => changed.includes(key);
    const pathInput = (key: 'dataPath' | 'cfgFile', ids: RowIds) => {
        const problem = issue(key);
        return (
            <>
                <input
                    id={ids.control}
                    className={cn(inputClass, 'font-mono text-ui')}
                    aria-invalid={problem ? true : undefined}
                    aria-describedby={problem ? `${ids.control}-error ${ids.about}` : ids.about}
                    value={fields[key]}
                    spellCheck={false}
                    autoCapitalize="none"
                    autoComplete="off"
                    onChange={(event) => {
                        edit(key, event.target.value);
                        forget(key);
                    }}
                />
                {problem && <FieldError id={`${ids.control}-error`}>{problem}</FieldError>}
            </>
        );
    };
    const switchRow = (key: 'autoStart' | 'txadminEvents' | 'hangDetection') => (
        <SettingRow id={key} field changed={is(key)}>
            {(ids) => (
                <Switch
                    id={ids.control}
                    aria-describedby={ids.about}
                    checked={fields[key]}
                    onChange={(event) => edit(key, event.target.checked)}
                />
            )}
        </SettingRow>
    );

    return (
        <div ref={form} className="flex flex-col gap-4">
            {saved === null && (
                <Notice tone="warning">
                    {t.server.unsaved}{' '}
                    <Link to="/server/setup" className="font-medium underline underline-offset-3">
                        {words.runSetup}
                    </Link>
                </Notice>
            )}
            {stale && outcome === null && <Notice tone="warning">{t.server.changedElsewhere}</Notice>}
            {saved?.legacyBuildPath != null && (
                <Notice tone="warning">
                    {t.server.legacyBuild}{' '}
                    <span className="font-mono break-all">{saved.legacyBuildPath}</span>
                </Notice>
            )}
            {outcome && (
                <Notice ref={notice} tabIndex={-1} tone={outcome.tone}>
                    {outcome.message}
                </Notice>
            )}

            <SettingsGroup title={words.groups.files}>
                <SettingRow id="dataPath" field changed={is('dataPath')}>
                    {(ids) => pathInput('dataPath', ids)}
                </SettingRow>
                <SettingRow id="cfgFile" field changed={is('cfgFile')}>
                    {(ids) => pathInput('cfgFile', ids)}
                </SettingRow>
                <SettingRow id="build">{() => <SelectedBuild />}</SettingRow>
            </SettingsGroup>

            <SettingsGroup title={words.groups.game}>
                <SettingRow id="onesync" changed={is('onesync')}>
                    {(ids) => (
                        <OneSyncChoice
                            ids={ids}
                            value={fields.onesync}
                            onChange={(mode) => edit('onesync', mode)}
                        />
                    )}
                </SettingRow>
                {switchRow('autoStart')}
                {switchRow('txadminEvents')}
            </SettingsGroup>

            <SettingsGroup title={words.groups.hang}>
                {switchRow('hangDetection')}
                {/* Left editable with detection off: the delay is kept for when it is back on. */}
                <SettingRow id="hangTimeout" field changed={is('hangTimeoutSeconds')}>
                    {(ids) => {
                        const problem = issue('hangTimeoutSeconds');
                        return (
                            <>
                                <input
                                    id={ids.control}
                                    type="number"
                                    inputMode="numeric"
                                    min={1}
                                    max={60}
                                    className={cn(inputClass, 'max-w-32')}
                                    aria-invalid={problem ? true : undefined}
                                    aria-describedby={
                                        problem ? `${ids.control}-error ${ids.about}` : ids.about
                                    }
                                    value={minutes}
                                    onChange={(event) => {
                                        setMinutes(event.target.value);
                                        setOutcome(null);
                                        forget('hangTimeoutSeconds');
                                    }}
                                />
                                {problem && <FieldError id={`${ids.control}-error`}>{problem}</FieldError>}
                            </>
                        );
                    }}
                </SettingRow>
            </SettingsGroup>

            <section className="card overflow-hidden" aria-labelledby={`${argumentsId}-title`}>
                <h3 id={`${argumentsId}-title`}>
                    <button
                        type="button"
                        className="flex min-h-13 w-full items-center gap-2.5 px-5 py-3 text-left hover:bg-hover focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-accent max-md:px-4"
                        aria-expanded={advanced}
                        aria-controls={argumentsId}
                        onClick={() => setAdvanced((open) => !open)}
                    >
                        <span className="text-card text-ink">{words.advanced}</span>
                        <span className="text-meta text-ink-3">{words.argumentsSet(rows.length)}</span>
                        <ChevronDown
                            size={16}
                            strokeWidth={1.5}
                            aria-hidden="true"
                            className={cn(
                                'ml-auto text-ink-3 transition-transform',
                                advanced && 'rotate-180',
                            )}
                        />
                    </button>
                </h3>
                <div id={argumentsId} hidden={!advanced} className="border-t border-line">
                    <SettingRow id="extraArguments" changed={is('extraArguments')}>
                        {(ids) => (
                            <fieldset aria-labelledby={ids.label} className="min-w-0 flex flex-col gap-3">
                                {rows.length === 0 ? (
                                    <p className="text-ui text-ink-3">{t.server.noArguments}</p>
                                ) : (
                                    <div
                                        className="grid grid-cols-[auto_minmax(0,12rem)_minmax(0,1fr)_auto] gap-x-2 text-small font-medium text-ink-3 max-md:hidden"
                                        aria-hidden="true"
                                    >
                                        <span className="w-9" />
                                        <span>{t.server.argumentName}</span>
                                        <span>{t.server.argumentValue}</span>
                                        <span className="w-8" />
                                    </div>
                                )}
                                {rows.map((row, index) => (
                                    <ArgumentRow
                                        key={row.id}
                                        row={row}
                                        position={index + 1}
                                        nameError={issue('argumentName', index)}
                                        valueError={issue('argumentValue', index)}
                                        nameRef={(input) => {
                                            if (input !== null) {
                                                nameInputs.current.set(row.id, input);
                                            }
                                            return () => {
                                                nameInputs.current.delete(row.id);
                                            };
                                        }}
                                        onChange={(change) => {
                                            editRow(row.id, change);
                                            forget(
                                                change.name === undefined ? 'argumentValue' : 'argumentName',
                                                index,
                                            );
                                        }}
                                        onRemove={() => {
                                            const next = rows[index + 1] ?? rows[index - 1];
                                            focusAfterRemoval.current = next?.id ?? 'add';
                                            setRows((current) =>
                                                current.filter((other) => other.id !== row.id),
                                            );
                                            forgetArguments();
                                            setOutcome(null);
                                        }}
                                    />
                                ))}
                                {issue('extraArguments') && (
                                    <FieldError>{issue('extraArguments')}</FieldError>
                                )}
                                <Button
                                    ref={addButton}
                                    tone="ghost"
                                    size="sm"
                                    icon={Plus}
                                    className="self-start"
                                    onClick={() => {
                                        setRows((current) => [
                                            ...current,
                                            { id: ++lastRow, name: '', value: '' },
                                        ]);
                                        setOutcome(null);
                                    }}
                                >
                                    {t.server.addArgument}
                                </Button>
                            </fieldset>
                        )}
                    </SettingRow>
                </div>
            </section>
        </div>
    );
}

/**
 * The three modes of OneSync, each with what it means for the server: the owner chooses between
 * slots and how the world is synced, not between three words.
 */
function OneSyncChoice({
    ids,
    value,
    onChange,
}: {
    ids: RowIds;
    value: OneSync;
    onChange: (mode: OneSync) => void;
}) {
    const { t } = useI18n();
    const modes = t.settings.launch.onesync;
    return (
        <div
            role="radiogroup"
            aria-labelledby={ids.label}
            aria-describedby={ids.about}
            className="flex flex-col gap-2"
        >
            {ONESYNC_MODES.map((mode) => (
                <label
                    key={mode}
                    className="grid cursor-pointer grid-cols-[auto_minmax(0,1fr)] items-start gap-x-2.5 gap-y-0.5 rounded-box border border-line-strong px-3 py-2.5 transition-colors hover:bg-hover has-checked:border-accent has-checked:bg-accent-soft has-focus-visible:outline-2 has-focus-visible:outline-offset-2 has-focus-visible:outline-accent"
                >
                    <input
                        type="radio"
                        name={`${ids.control}-onesync`}
                        value={mode}
                        className="peer sr-only"
                        checked={value === mode}
                        aria-labelledby={`${ids.control}-${mode}-name`}
                        aria-describedby={`${ids.control}-${mode}`}
                        onChange={() => onChange(mode)}
                    />
                    <span
                        aria-hidden="true"
                        className="row-span-2 mt-0.5 grid size-4 place-items-center rounded-pill border border-line-strong bg-surface peer-checked:border-btn peer-checked:bg-btn peer-checked:[&>span]:opacity-100"
                    >
                        <span className="size-1.5 rounded-pill bg-btn-ink opacity-0" />
                    </span>
                    <span id={`${ids.control}-${mode}-name`} className="text-body font-medium text-ink">
                        {modes[mode].name}
                    </span>
                    <span id={`${ids.control}-${mode}`} className="text-small text-ink-2">
                        {modes[mode].about}
                    </span>
                </label>
            ))}
        </div>
    );
}

/** Which build FXServer starts, chosen on the Artifacts page rather than here. */
function SelectedBuild() {
    const { t } = useI18n();
    const { status, refused } = useArtifacts();
    const text =
        status === null
            ? refused
                ? null
                : t.server.buildUnknown
            : status.selected === null
              ? t.server.buildNone
              : t.server.buildSelected(String(status.selected));
    return (
        <p className="flex flex-wrap items-center gap-x-3 gap-y-1 text-body text-ink-2">
            {text}
            <Link
                to="/artifacts"
                className="inline-flex items-center gap-1.5 font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
            >
                {t.settings.launch.openArtifacts}
                <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
            </Link>
        </p>
    );
}

function ArgumentRow({
    row,
    position,
    nameError,
    valueError,
    nameRef,
    onChange,
    onRemove,
}: {
    row: Row;
    position: number;
    nameError: string | undefined;
    valueError: string | undefined;
    nameRef: Ref<HTMLInputElement>;
    onChange: (change: Partial<Row>) => void;
    onRemove: () => void;
}) {
    const { t } = useI18n();
    const id = useId();

    // The elements come in the order they read on screen, so the keyboard follows the eye: the
    // name, its message, the value, its message, then the button, beside the value on phones and
    // at the end of the line on wider screens. Each message sits under its own field.
    return (
        <fieldset className="min-w-0">
            <legend className="sr-only">{t.server.argument(position)}</legend>
            <div className="grid grid-flow-row-dense grid-cols-[minmax(0,1fr)_auto] items-center gap-2 md:grid-cols-[auto_minmax(0,12rem)_minmax(0,1fr)_auto]">
                <span
                    className="hidden w-9 font-mono text-ui text-ink-3 md:col-start-1 md:row-start-1 md:block"
                    aria-hidden="true"
                >
                    +set
                </span>
                <input
                    ref={nameRef}
                    className={cn(
                        inputClass,
                        'col-span-2 font-mono text-ui md:col-span-1 md:col-start-2 md:row-start-1',
                    )}
                    aria-label={t.server.argumentName}
                    aria-invalid={nameError ? true : undefined}
                    aria-describedby={nameError ? `${id}-name-error` : undefined}
                    value={row.name}
                    spellCheck={false}
                    autoCapitalize="none"
                    autoComplete="off"
                    onChange={(event) => onChange({ name: event.target.value })}
                />
                {nameError && (
                    <div className="col-span-2 md:col-span-1 md:col-start-2 md:row-start-2">
                        <FieldError id={`${id}-name-error`}>{nameError}</FieldError>
                    </div>
                )}
                <input
                    className={cn(inputClass, 'font-mono text-ui md:col-start-3 md:row-start-1')}
                    aria-label={t.server.argumentValue}
                    aria-invalid={valueError ? true : undefined}
                    aria-describedby={valueError ? `${id}-value-error` : undefined}
                    value={row.value}
                    spellCheck={false}
                    autoCapitalize="none"
                    autoComplete="off"
                    onChange={(event) => onChange({ value: event.target.value })}
                />
                {valueError && (
                    <div className="col-span-2 md:col-span-1 md:col-start-3 md:row-start-2">
                        <FieldError id={`${id}-value-error`}>{valueError}</FieldError>
                    </div>
                )}
                <button
                    type="button"
                    className={cn(
                        buttonVariants({ tone: 'ghost', size: 'icon-sm' }),
                        'md:col-start-4 md:row-start-1',
                    )}
                    aria-label={t.server.removeArgument(position)}
                    onClick={onRemove}
                >
                    <X size={14} strokeWidth={1.5} aria-hidden="true" />
                </button>
            </div>
        </fieldset>
    );
}
