import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { FolderOpen, Plus, Power, RotateCw, SquareTerminal, X, type LucideIcon } from 'lucide-react';
import { useId, useState, type FormEvent, type ReactNode } from 'react';

import { ApiError } from '../api/client';
import type { LaunchField } from '../api/generated/LaunchField';
import type { LaunchIssue } from '../api/generated/LaunchIssue';
import type { LaunchSettings } from '../api/generated/LaunchSettings';
import type { OneSync } from '../api/generated/OneSync';
import type { StoredLaunch } from '../api/generated/StoredLaunch';
import { cleaned, countChanges, DEFAULT_LAUNCH, launchIssues, launchQuery, saveLaunch } from '../api/launch';
import { Button, buttonVariants } from '../components/Button';
import { Checkbox } from '../components/Checkbox';
import { FieldError, TextField, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

const ONESYNC_MODES: OneSync[] = ['off', 'on', 'legacy'];

/** An extra argument as the form edits it: the id keeps a row's fields when one above is removed. */
type Row = { id: number; name: string; value: string };

type Outcome = { tone: 'success' | 'warning' | 'danger'; message: string };

let lastRow = 0;
const toRows = (settings: LaunchSettings): Row[] =>
    settings.extraArguments.map((argument) => ({ id: ++lastRow, ...argument }));

export function ServerPage() {
    const { t } = useI18n();
    const launch = useQuery(launchQuery);

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.server.title} <span className="gloss">{t.server.gloss}</span>
                </h1>
                <span className="inline-flex items-center gap-1.5 text-ui text-ink-3">
                    <RotateCw size={14} strokeWidth={1.5} aria-hidden="true" />
                    {t.server.nextStart}
                </span>
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
    const [issues, setIssues] = useState<LaunchIssue[]>([]);
    const [outcome, setOutcome] = useState<Outcome | null>(null);

    const draft: LaunchSettings = {
        ...fields,
        extraArguments: rows.map(({ name, value }) => ({ name, value })),
    };
    const changes = countChanges(baseline, draft);
    const stale = (saved?.version ?? null) !== (shown?.version ?? null);

    const startFrom = (stored: StoredLaunch | null) => {
        const settings = stored?.settings ?? DEFAULT_LAUNCH;
        setShown(stored);
        setFields(settings);
        setRows(toRows(settings));
    };

    // Another session saved since: with nothing typed yet, show what is stored now rather than
    // what it replaced, which a save would quietly bring back.
    if (stale && changes === 0) {
        startFrom(saved);
    }

    const save = useMutation({
        mutationFn: () => saveLaunch({ settings: cleaned(draft), version: shown?.version ?? null }),
        onSuccess: (stored) => {
            queryClient.setQueryData(launchQuery.queryKey, stored);
            startFrom(stored);
            setIssues([]);
            setOutcome({ tone: 'success', message: t.server.saved });
        },
        onError: (error) => {
            const conflict = error instanceof ApiError && error.code === 'conflict';
            if (conflict) {
                void queryClient.invalidateQueries({ queryKey: launchQuery.queryKey });
            }
            setIssues(
                error instanceof ApiError && error.code === 'invalid_launch_settings'
                    ? launchIssues(error.params)
                    : [],
            );
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
        setOutcome(null);
    };

    const submit = (event: FormEvent) => {
        event.preventDefault();
        setOutcome(null);
        if (!save.isPending) {
            save.mutate();
        }
    };

    return (
        <form className="flex flex-col gap-3" onSubmit={submit} noValidate>
            {saved === null && <Notice tone="warning">{t.server.unsaved}</Notice>}
            {stale && outcome === null && <Notice tone="warning">{t.server.changedElsewhere}</Notice>}
            {outcome && <Notice tone={outcome.tone}>{outcome.message}</Notice>}

            <Card id="server-files" icon={FolderOpen} title={t.server.files}>
                <div className="flex max-w-2xl flex-col gap-4">
                    <PathField
                        label={t.server.buildPath}
                        hint={t.server.buildPathHint}
                        value={fields.buildPath}
                        error={issue('buildPath')}
                        onChange={(value) => {
                            edit('buildPath', value);
                            forget('buildPath');
                        }}
                    />
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
                            onChange={(change) => {
                                editRow(row.id, change);
                                forget(change.name === undefined ? 'argumentValue' : 'argumentName', index);
                            }}
                            onRemove={() => {
                                setRows((current) => current.filter((other) => other.id !== row.id));
                                forgetArguments();
                                setOutcome(null);
                            }}
                        />
                    ))}
                    {issue('extraArguments') && <FieldError>{issue('extraArguments')}</FieldError>}
                    <Button
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

function Card({
    id,
    icon: Icon,
    title,
    meta,
    children,
}: {
    id: string;
    icon: LucideIcon;
    title: string;
    meta?: string;
    children: ReactNode;
}) {
    return (
        <section className="card" aria-labelledby={id}>
            <div className="card-head">
                <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                    <Icon size={14} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <h2 id={id} className="text-card text-ink">
                    {title}
                </h2>
                {meta && <p className="text-meta text-ink-3">{meta}</p>}
            </div>
            <div className="p-5 max-md:p-4">{children}</div>
        </section>
    );
}

function PathField({
    label,
    hint,
    value,
    error,
    onChange,
}: {
    label: string;
    hint: string;
    value: string;
    error: string | undefined;
    onChange: (value: string) => void;
}) {
    return (
        <TextField
            label={label}
            hint={hint}
            value={value}
            error={error}
            spellCheck={false}
            autoCapitalize="none"
            autoComplete="off"
            inputClassName="font-mono text-ui"
            onChange={(event) => onChange(event.target.value)}
        />
    );
}

function OneSyncChoice({ value, onChange }: { value: OneSync; onChange: (mode: OneSync) => void }) {
    const { t } = useI18n();
    const hint = useId();
    return (
        <fieldset className="flex min-w-0 flex-col gap-1.5" aria-describedby={hint}>
            <legend className="mb-1.5 text-ui font-medium text-ink">{t.server.onesync}</legend>
            <div className="inline-flex max-w-full gap-0.5 self-start rounded-control border border-line bg-surface-2 p-0.75">
                {ONESYNC_MODES.map((mode) => (
                    <label
                        key={mode}
                        className="inline-flex h-6.5 cursor-pointer items-center justify-center rounded-[7px] px-2.5 text-small font-medium text-ink-3 transition-colors hover:text-ink has-checked:bg-raise has-checked:text-ink has-checked:shadow-[var(--elev-sm),0_0_0_1px_var(--line)] has-focus-visible:outline-2 has-focus-visible:outline-offset-1 has-focus-visible:outline-accent max-md:h-11 max-md:min-w-16"
                    >
                        <input
                            type="radio"
                            name="onesync"
                            value={mode}
                            className="sr-only"
                            checked={value === mode}
                            onChange={() => onChange(mode)}
                        />
                        {t.server.onesyncModes[mode]}
                    </label>
                ))}
            </div>
            <p id={hint} className="text-small text-ink-3">
                {t.server.onesyncHint}
            </p>
        </fieldset>
    );
}

function CheckRow({
    label,
    description,
    checked,
    onChange,
}: {
    label: string;
    description: string;
    checked: boolean;
    onChange: (checked: boolean) => void;
}) {
    const id = useId();
    return (
        <label
            htmlFor={`${id}-box`}
            className="flex cursor-pointer items-start gap-3 rounded-sm px-2 py-2 hover:bg-hover"
        >
            <Checkbox
                id={`${id}-box`}
                checked={checked}
                aria-labelledby={`${id}-name`}
                aria-describedby={`${id}-about`}
                onChange={(event) => onChange(event.target.checked)}
            />
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <span id={`${id}-name`} className="text-body font-medium text-ink">
                    {label}
                </span>
                <span id={`${id}-about`} className="text-small text-ink-2">
                    {description}
                </span>
            </span>
        </label>
    );
}

function ArgumentRow({
    row,
    position,
    nameError,
    valueError,
    onChange,
    onRemove,
}: {
    row: Row;
    position: number;
    nameError: string | undefined;
    valueError: string | undefined;
    onChange: (change: Partial<Row>) => void;
    onRemove: () => void;
}) {
    const { t } = useI18n();
    const id = useId();
    const errors = [nameError && `${id}-name-error`, valueError && `${id}-value-error`].filter(Boolean);

    return (
        <fieldset className="flex min-w-0 flex-col gap-1.5">
            <legend className="sr-only">{t.server.argument(position)}</legend>
            <div className="grid grid-cols-[auto_minmax(0,16rem)_minmax(0,1fr)_auto] items-center gap-2 max-md:grid-cols-[minmax(0,1fr)_auto]">
                <span className="w-9 font-mono text-ui text-ink-3 max-md:hidden" aria-hidden="true">
                    +set
                </span>
                <input
                    className={cn(inputClass, 'font-mono text-ui')}
                    aria-label={t.server.argumentName}
                    aria-invalid={nameError ? true : undefined}
                    aria-describedby={nameError ? `${id}-name-error` : undefined}
                    value={row.name}
                    spellCheck={false}
                    autoCapitalize="none"
                    autoComplete="off"
                    onChange={(event) => onChange({ name: event.target.value })}
                />
                <input
                    className={cn(inputClass, 'font-mono text-ui max-md:order-last max-md:col-span-2')}
                    aria-label={t.server.argumentValue}
                    aria-invalid={valueError ? true : undefined}
                    aria-describedby={valueError ? `${id}-value-error` : undefined}
                    value={row.value}
                    spellCheck={false}
                    autoCapitalize="none"
                    autoComplete="off"
                    onChange={(event) => onChange({ value: event.target.value })}
                />
                <button
                    type="button"
                    className={buttonVariants({ tone: 'ghost', size: 'icon-sm' })}
                    aria-label={t.server.removeArgument(position)}
                    onClick={onRemove}
                >
                    <X size={14} strokeWidth={1.5} aria-hidden="true" />
                </button>
            </div>
            {errors.length > 0 && (
                <div className="flex flex-col gap-1 md:pl-11">
                    {nameError && <FieldError id={`${id}-name-error`}>{nameError}</FieldError>}
                    {valueError && <FieldError id={`${id}-value-error`}>{valueError}</FieldError>}
                </div>
            )}
        </fieldset>
    );
}
