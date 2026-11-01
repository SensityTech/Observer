import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import {
    ArrowRight,
    FolderOpen,
    FolderSearch,
    HeartPulse,
    Plus,
    Power,
    RotateCw,
    SquareTerminal,
    X,
} from 'lucide-react';
import { useEffect, useId, useRef, useState, type FormEvent, type Ref } from 'react';

import { ApiError } from '../api/client';
import type { LaunchField } from '../api/generated/LaunchField';
import type { LaunchIssue } from '../api/generated/LaunchIssue';
import type { LaunchSettings } from '../api/generated/LaunchSettings';
import type { StoredLaunch } from '../api/generated/StoredLaunch';
import {
    changedSettings,
    cleaned,
    countChanges,
    DEFAULT_LAUNCH,
    HANG_SETTINGS,
    launchIssues,
    launchQuery,
    saveLaunch,
} from '../api/launch';
import { useArtifacts } from '../artifacts/context';
import { Button, buttonVariants } from '../components/Button';
import { Card } from '../components/Card';
import { CheckRow } from '../components/CheckRow';
import { FieldError, inputClass, TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { OneSyncChoice } from '../components/OneSyncChoice';
import { PathField } from '../components/PathField';
import type { Messages } from '../i18n/en';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

/** An extra argument as the form edits it: the id keeps a row's fields when one above is removed. */
type Row = { id: number; name: string; value: string };

type Outcome = { tone: 'neutral' | 'success' | 'warning' | 'danger'; message: string };

let lastRow = 0;
const toRows = (settings: LaunchSettings): Row[] =>
    settings.extraArguments.map((argument) => ({ id: ++lastRow, ...argument }));

/**
 * The hang timeout as the form edits it, in minutes. Observer takes whole minutes only, but a value
 * stored otherwise reads 8.33 rather than 8.333333333333334.
 */
const minutesOf = (settings: LaunchSettings) =>
    String(Math.round((settings.hangTimeoutSeconds / 60) * 100) / 100);
/** The minutes typed as they are sent, in seconds: rounded, as 0.1 × 60 makes 6.000000000000001. */
const secondsOf = (minutes: string) =>
    minutes.trim() === '' ? Number.NaN : Math.round(Number(minutes) * 60);
/** A whole number of minutes from 1 to 60, as Observer takes it in seconds. */
const minutesFit = (minutes: string) => {
    const value = Number(minutes);
    return minutes.trim() !== '' && Number.isInteger(value) && value >= 1 && value <= 60;
};

/** What a save says: hang detection reaches a running server at once, the rest its next start. */
const savedMessage = (t: Messages, before: LaunchSettings, after: LaunchSettings) => {
    const changed = changedSettings(before, after);
    const hang = changed.some((key) => HANG_SETTINGS.has(key));
    if (!hang) {
        return t.server.saved;
    }
    return changed.every((key) => HANG_SETTINGS.has(key)) ? t.server.savedHang : t.server.savedBoth;
};

export function ServerPage() {
    const { t } = useI18n();
    const launch = useQuery(launchQuery);

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.server.title} <span className="gloss">{t.server.gloss}</span>
                </h1>
                <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
                    <span className="inline-flex items-center gap-1.5 text-ui text-ink-3">
                        <RotateCw size={14} strokeWidth={1.5} aria-hidden="true" />
                        {t.server.nextStart}
                    </span>
                    <Link to="/server/setup" className={buttonVariants({ size: 'sm' })}>
                        <FolderSearch size={14} strokeWidth={1.5} aria-hidden="true" />
                        {t.server.runSetup}
                    </Link>
                </div>
            </div>
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
            {launch.data !== undefined && <LaunchForm saved={launch.data} />}
        </div>
    );
}

function LaunchForm({ saved }: { saved: StoredLaunch | null }) {
    const { t } = useI18n();
    const queryClient = useQueryClient();
    // What the draft was started from: the settings as stored then, and the version a save names.
    const [shown, setShown] = useState(saved);
    const baseline = shown?.settings ?? DEFAULT_LAUNCH;
    const [fields, setFields] = useState<LaunchSettings>(baseline);
    const [rows, setRows] = useState<Row[]>(() => toRows(baseline));
    const [minutes, setMinutes] = useState(() => minutesOf(baseline));
    const [issues, setIssues] = useState<LaunchIssue[]>([]);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const notice = useRef<HTMLDivElement>(null);
    const nameInputs = useRef(new Map<number, HTMLInputElement>());
    const addButton = useRef<HTMLButtonElement>(null);
    /** Where the keyboard goes once a removed row is gone: another row's name, or the add button. */
    const focusAfterRemoval = useRef<number | 'add' | null>(null);
    const form = useRef<HTMLFormElement>(null);
    const showFirstIssue = useRef(false);

    const draft: LaunchSettings = {
        ...fields,
        extraArguments: rows.map(({ name, value }) => ({ name, value })),
        // Compared in the minutes the field shows: left as it was, it gives back the seconds stored,
        // so one the field cannot show exactly is never an unsaved change nobody made.
        hangTimeoutSeconds:
            minutes === minutesOf(baseline) ? baseline.hangTimeoutSeconds : secondsOf(minutes),
    };
    const changes = countChanges(baseline, draft);
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
    if (stale && changes === 0) {
        startFrom(saved);
    }

    // What the save started from goes with it: by the time it answers, the form may show another.
    const save = useMutation({
        mutationFn: ({ settings }: { settings: LaunchSettings; before: LaunchSettings }) =>
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
            setIssues(refused ? launchIssues(error.params) : []);
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

    const edit = <K extends keyof LaunchSettings>(key: K, value: LaunchSettings[K]) => {
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

    const discard = () => {
        startFrom(saved);
        setIssues([]);
        setOutcome({ tone: 'neutral', message: t.server.discarded });
    };

    // Saving or discarding takes the save bar away, and with it the button the keyboard was on:
    // it moves to the notice saying what happened rather than back to the top of the document.
    useEffect(() => {
        if (outcome !== null && document.activeElement === document.body) {
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

    const submit = (event: FormEvent) => {
        event.preventDefault();
        setOutcome(null);
        // A fraction or an emptied field would be refused as a malformed request rather than next
        // to its field: it is checked before sending.
        if (!minutesFit(minutes)) {
            showFirstIssue.current = true;
            setIssues((current) => [
                ...current.filter((each) => each.field !== 'hangTimeoutSeconds'),
                { field: 'hangTimeoutSeconds', index: null, problem: 'outOfRange' },
            ]);
            return;
        }
        if (!save.isPending) {
            save.mutate({ settings: cleaned(draft), before: baseline });
        }
    };

    return (
        <form ref={form} className="flex flex-col gap-3" onSubmit={submit} noValidate>
            {saved === null && (
                <Notice tone="warning">
                    {t.server.unsaved}{' '}
                    <Link to="/server/setup" className="font-medium underline underline-offset-3">
                        {t.server.runSetup}
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

            <Card id="server-files" icon={FolderOpen} title={t.server.files}>
                <div className="flex max-w-2xl flex-col gap-4">
                    <SelectedBuild />
                    <PathField
                        label={t.server.dataPath}
                        hint={t.server.dataPathHint}
                        value={fields.dataPath}
                        error={issue('dataPath')}
                        onChange={(value) => {
                            edit('dataPath', value);
                            forget('dataPath');
                        }}
                    />
                    <PathField
                        label={t.server.cfgFile}
                        hint={t.server.cfgFileHint}
                        value={fields.cfgFile}
                        error={issue('cfgFile')}
                        onChange={(value) => {
                            edit('cfgFile', value);
                            forget('cfgFile');
                        }}
                    />
                </div>
            </Card>

            <Card id="server-start" icon={Power} title={t.server.start}>
                <div className="flex max-w-2xl flex-col gap-5">
                    <OneSyncChoice value={fields.onesync} onChange={(mode) => edit('onesync', mode)} />
                    <div className="-mx-2 flex flex-col gap-1">
                        <CheckRow
                            label={t.server.autoStart}
                            description={t.server.autoStartHint}
                            checked={fields.autoStart}
                            onChange={(checked) => edit('autoStart', checked)}
                        />
                        <CheckRow
                            label={t.server.txadminEvents}
                            description={t.server.txadminEventsHint}
                            checked={fields.txadminEvents}
                            onChange={(checked) => edit('txadminEvents', checked)}
                        />
                    </div>
                </div>
            </Card>

            <Card id="server-hang" icon={HeartPulse} title={t.server.hang} meta={t.server.hangMeta}>
                <div className="flex max-w-2xl flex-col gap-4">
                    <div className="-mx-2 flex flex-col gap-1">
                        <CheckRow
                            label={t.server.hangDetection}
                            description={t.server.hangDetectionHint}
                            checked={fields.hangDetection}
                            onChange={(checked) => edit('hangDetection', checked)}
                        />
                    </div>
                    {/* Left editable with detection off: the delay is kept for when it is back on. */}
                    <TextField
                        label={t.server.hangTimeout}
                        hint={t.server.hangTimeoutHint}
                        type="number"
                        inputMode="numeric"
                        min={1}
                        max={60}
                        className="max-w-48"
                        value={minutes}
                        error={issue('hangTimeoutSeconds')}
                        onChange={(event) => {
                            setMinutes(event.target.value);
                            setOutcome(null);
                            forget('hangTimeoutSeconds');
                        }}
                    />
                </div>
            </Card>

            <Card
                id="server-arguments"
                icon={SquareTerminal}
                title={t.server.arguments}
                meta={t.server.argumentsMeta}
            >
                <div className="flex flex-col gap-3">
                    {rows.length === 0 ? (
                        <p className="text-ui text-ink-3">{t.server.noArguments}</p>
                    ) : (
                        <div
                            className="grid grid-cols-[auto_minmax(0,16rem)_minmax(0,1fr)_auto] gap-x-2 text-small font-medium text-ink-3 max-md:hidden"
                            aria-hidden="true"
                        >
                            <span className="w-9" />
                            <span>{t.server.argumentName}</span>
                            <span>{t.server.argumentValue}</span>
                            <span className="w-9" />
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
                                forget(change.name === undefined ? 'argumentValue' : 'argumentName', index);
                            }}
                            onRemove={() => {
                                const next = rows[index + 1] ?? rows[index - 1];
                                focusAfterRemoval.current = next?.id ?? 'add';
                                setRows((current) => current.filter((other) => other.id !== row.id));
                                forgetArguments();
                                setOutcome(null);
                            }}
                        />
                    ))}
                    {issue('extraArguments') && <FieldError>{issue('extraArguments')}</FieldError>}
                    <Button
                        ref={addButton}
                        tone="ghost"
                        size="sm"
                        icon={Plus}
                        className="self-start"
                        onClick={() => {
                            setRows((current) => [...current, { id: ++lastRow, name: '', value: '' }]);
                            setOutcome(null);
                        }}
                    >
                        {t.server.addArgument}
                    </Button>
                </div>
            </Card>

            {(saved === null || changes > 0) && (
                <div className="sticky bottom-3 z-10 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-box border border-line-strong bg-surface px-3.5 py-2.5 shadow-pop">
                    <output className="mr-auto text-ui text-ink-2">
                        {saved === null ? t.server.notSaved : t.server.changes(changes)}
                    </output>
                    {saved !== null && (
                        <Button tone="ghost" onClick={discard} className="max-md:flex-1">
                            {t.server.discard}
                        </Button>
                    )}
                    <Button type="submit" tone="ink" pending={save.isPending} className="max-md:flex-1">
                        {t.server.save}
                    </Button>
                </div>
            )}
        </form>
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
        <div className="flex flex-col gap-1">
            <p className="text-ui font-medium text-ink">{t.server.build}</p>
            <p className="flex flex-wrap items-center gap-x-3 gap-y-1 text-body text-ink-2">
                {text}
                <Link
                    to="/artifacts"
                    className="inline-flex items-center gap-1.5 font-medium text-ink underline-offset-2 hover:underline"
                >
                    {t.server.openArtifacts}
                    <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                </Link>
            </p>
        </div>
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
            <div className="grid grid-flow-row-dense grid-cols-[minmax(0,1fr)_auto] items-center gap-2 md:grid-cols-[auto_minmax(0,16rem)_minmax(0,1fr)_auto]">
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
