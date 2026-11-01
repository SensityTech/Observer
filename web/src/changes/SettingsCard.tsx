import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ScanSearch, Settings2, TriangleAlert } from 'lucide-react';
import { useId, useState } from 'react';

import { changesSettingsQuery, saveChangesSettings, verifyFiles } from '../api/changes';
import { ApiError } from '../api/client';
import type { ChangesSettings } from '../api/generated/ChangesSettings';
import type { ChangesSettingsView } from '../api/generated/ChangesSettingsView';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { CheckRow } from '../components/CheckRow';
import { FieldError, inputClass, TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { stringParam } from '../i18n/params';
import { cn } from '../lib/cn';
import { GB, MB, MIB } from './units';

type Draft = {
    enabled: boolean;
    extensions: string;
    excluded: string;
    hidden: string;
    maxText: string;
    budget: string;
    retentionDays: string;
    hashRate: string;
    verifyDays: string;
    startEvent: boolean;
};

type NumberField = 'maxText' | 'budget' | 'retentionDays' | 'hashRate' | 'verifyDays';

/** Each number as typed, its unit, and the range Observer accepts, in that unit. */
const NUMBERS = {
    maxText: { unit: MIB, min: 1024 / MIB, max: 8 },
    budget: { unit: GB, min: 0, max: 1000 },
    retentionDays: { unit: 1, min: 1, max: 3650 },
    hashRate: { unit: MB, min: 1, max: 10_000 },
    verifyDays: { unit: 1, min: 0, max: 365 },
} satisfies Record<NumberField, { unit: number; min: number; max: number }>;

const lines = (text: string) =>
    text
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line !== '');

const toDraft = (settings: ChangesSettings): Draft => ({
    enabled: settings.enabled,
    extensions: settings.extensions.join(' '),
    excluded: settings.excluded.join('\n'),
    hidden: settings.hidden.join('\n'),
    maxText: String(settings.maxText / MIB),
    budget: String(settings.budget / GB),
    retentionDays: String(settings.retentionDays),
    hashRate: String(settings.hashRate / MB),
    verifyDays: String(settings.verifyDays),
    startEvent: settings.startEvent,
});

/** The settings the draft says, or the fields out of range. */
const fromDraft = (draft: Draft, saved: ChangesSettings): ChangesSettings | NumberField[] => {
    const wrong = (Object.keys(NUMBERS) as NumberField[]).filter((field) => {
        const value = Number(draft[field]);
        const { min, max } = NUMBERS[field];
        return draft[field].trim() === '' || !Number.isFinite(value) || value < min || value > max;
    });
    if (wrong.length > 0) {
        return wrong;
    }
    const scaled = (field: NumberField) => Math.round(Number(draft[field]) * NUMBERS[field].unit);
    return {
        ...saved,
        enabled: draft.enabled,
        extensions: draft.extensions.split(/[\s,]+/).filter((extension) => extension !== ''),
        excluded: lines(draft.excluded),
        hidden: lines(draft.hidden),
        maxText: scaled('maxText'),
        budget: scaled('budget'),
        retentionDays: scaled('retentionDays'),
        hashRate: scaled('hashRate'),
        verifyDays: scaled('verifyDays'),
        startEvent: draft.startEvent,
    };
};

/** The owner's settings of change tracking, saved with the password asked again. */
export function SettingsCard() {
    const { t } = useI18n();
    const settings = useQuery(changesSettingsQuery);
    if (settings.isError) {
        return <Notice tone="danger">{t.changes.settings.loadFailed}</Notice>;
    }
    if (settings.data === undefined) {
        return (
            <div
                className="skeleton h-64 w-full rounded-card"
                aria-busy="true"
                aria-label={t.common.loading}
            />
        );
    }
    return <SettingsForm view={settings.data} />;
}

function SettingsForm({ view }: { view: ChangesSettingsView }) {
    const { t, format } = useI18n();
    const words = t.changes.settings;
    const id = useId();
    const queryClient = useQueryClient();
    const [draft, setDraft] = useState(() => toDraft(view.settings));
    // What is wrong with each field, by its name in the settings.
    const [problems, setProblems] = useState<Partial<Record<string, string>>>({});
    const [outcome, setOutcome] = useState<{ tone: 'success' | 'danger'; message: string } | null>(null);
    const saved = toDraft(view.settings);
    const changes = (Object.keys(draft) as (keyof Draft)[]).filter((key) => draft[key] !== saved[key]).length;

    const save = useMutation({
        mutationFn: saveChangesSettings,
        onMutate: () => {
            setOutcome(null);
            setProblems({});
        },
        onSuccess: (next) => {
            queryClient.setQueryData(changesSettingsQuery.queryKey, next);
            setDraft(toDraft(next.settings));
            setOutcome({ tone: 'success', message: words.saved });
        },
        onError: (error) => {
            if (
                error instanceof ApiError &&
                (error.code === 'invalid_glob' || error.code === 'invalid_request')
            ) {
                const field = stringParam(error.params, 'field');
                setProblems({
                    [field]:
                        error.code === 'invalid_glob'
                            ? errorMessage(t, error)
                            : field === 'extensions'
                              ? words.extensionsInvalid
                              : words.outOfRange,
                });
            }
            setOutcome({ tone: 'danger', message: errorMessage(t, error) });
        },
    });
    const verify = useMutation({ mutationFn: verifyFiles });

    const submit = () => {
        const settings = fromDraft(draft, view.settings);
        if (Array.isArray(settings)) {
            setOutcome(null);
            setProblems(Object.fromEntries(settings.map((field) => [field, words.outOfRange])));
            return;
        }
        save.mutate(settings);
    };
    const number = (field: NumberField, label: string, hint?: string, extra?: string) => (
        <TextField
            label={label}
            hint={[hint, extra].filter((part) => part !== undefined).join(' ')}
            type="number"
            inputMode="decimal"
            min={NUMBERS[field].min}
            max={NUMBERS[field].max}
            className="max-w-56"
            value={draft[field]}
            error={problems[field]}
            onChange={(event) => setDraft({ ...draft, [field]: event.target.value })}
        />
    );

    return (
        <form
            className="flex flex-col gap-3"
            noValidate
            onSubmit={(event) => {
                event.preventDefault();
                if (!save.isPending) {
                    submit();
                }
            }}
        >
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            <Card
                id={`${id}-title`}
                icon={Settings2}
                title={words.title}
                meta={words.meta}
                aside={
                    <Button
                        size="sm"
                        icon={ScanSearch}
                        pending={verify.isPending}
                        onClick={() => verify.mutate()}
                    >
                        {words.verify}
                    </Button>
                }
            >
                <div className="flex max-w-3xl min-w-0 flex-col gap-5">
                    {verify.isSuccess && <Notice tone="success">{words.verifyStarted}</Notice>}
                    {verify.isError && <Notice tone="danger">{errorMessage(t, verify.error)}</Notice>}
                    <div className="-mx-2 flex flex-col">
                        <CheckRow
                            label={words.enabled}
                            description={words.enabledHint}
                            checked={draft.enabled}
                            onChange={(enabled) => setDraft({ ...draft, enabled })}
                        />
                        <CheckRow
                            label={words.startEvent}
                            description={words.startEventHint}
                            checked={draft.startEvent}
                            onChange={(startEvent) => setDraft({ ...draft, startEvent })}
                        />
                    </div>
                    <TextField
                        label={words.extensions}
                        hint={words.extensionsHint}
                        inputClassName="font-mono text-ui"
                        spellCheck={false}
                        autoCapitalize="none"
                        value={draft.extensions}
                        error={problems.extensions}
                        onChange={(event) => setDraft({ ...draft, extensions: event.target.value })}
                    />
                    <Globs
                        id={`${id}-excluded`}
                        label={words.excluded}
                        hint={words.excludedHint}
                        value={draft.excluded}
                        saved={view.settings.excluded}
                        matches={view.excludedMatches}
                        problem={problems.excluded}
                        onChange={(excluded) => setDraft({ ...draft, excluded })}
                    />
                    <Globs
                        id={`${id}-hidden`}
                        label={words.hidden}
                        hint={words.hiddenHint}
                        value={draft.hidden}
                        saved={view.settings.hidden}
                        matches={view.hiddenMatches}
                        problem={problems.hidden}
                        onChange={(hidden) => setDraft({ ...draft, hidden })}
                    />
                    <div className="grid grid-cols-1 gap-5 md:grid-cols-2">
                        {number('maxText', words.maxText, words.maxTextHint)}
                        {number(
                            'budget',
                            words.budget,
                            words.budgetHint,
                            words.stored(format.bytes(view.stored)),
                        )}
                        {number('retentionDays', words.retentionDays)}
                        {number('hashRate', words.hashRate, words.hashRateHint)}
                        {number('verifyDays', words.verifyDays, words.verifyDaysHint)}
                    </div>
                </div>
            </Card>
            {changes > 0 && (
                <div className="sticky bottom-3 z-10 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-box border border-line-strong bg-surface px-3.5 py-2.5 shadow-pop">
                    <output className="mr-auto text-ui text-ink-2">{words.changes(changes)}</output>
                    <Button
                        tone="ghost"
                        className="max-md:flex-1"
                        onClick={() => {
                            setDraft(saved);
                            setProblems({});
                            setOutcome(null);
                        }}
                    >
                        {words.discard}
                    </Button>
                    <Button type="submit" tone="ink" className="max-md:flex-1" pending={save.isPending}>
                        {words.save}
                    </Button>
                </div>
            )}
        </form>
    );
}

/** Patterns one a line, and what each saved one covers in the latest snapshot. */
function Globs({
    id,
    label,
    hint,
    value,
    saved,
    matches,
    problem,
    onChange,
}: {
    id: string;
    label: string;
    hint: string;
    value: string;
    saved: string[];
    matches: number[];
    problem: string | undefined;
    onChange: (value: string) => void;
}) {
    const { t } = useI18n();
    const words = t.changes.settings;
    return (
        <div className="flex min-w-0 flex-col gap-1.5">
            <label htmlFor={id} className="text-ui font-medium text-ink">
                {label}
            </label>
            <textarea
                id={id}
                rows={Math.min(8, Math.max(3, value.split('\n').length + 1))}
                spellCheck={false}
                autoCapitalize="none"
                aria-invalid={problem !== undefined || undefined}
                aria-describedby={problem === undefined ? `${id}-hint` : `${id}-error ${id}-hint`}
                className={cn(inputClass, 'h-auto min-h-24 resize-y py-2.5 font-mono text-ui leading-[1.55]')}
                value={value}
                onChange={(event) => onChange(event.target.value)}
            />
            {problem !== undefined && <FieldError id={`${id}-error`}>{problem}</FieldError>}
            <p id={`${id}-hint`} className="text-small text-ink-3">
                {hint}
            </p>
            {saved.length > 0 && (
                <ul className="flex flex-col gap-1">
                    {saved.map((glob, index) => {
                        const count = matches[index] ?? 0;
                        return (
                            <li
                                key={glob}
                                className="flex flex-wrap items-center gap-x-2 text-small text-ink-2"
                            >
                                <code className="code-inline">{glob}</code>
                                {count === 0 ? (
                                    <span className="inline-flex items-center gap-1 text-warning">
                                        <TriangleAlert size={12} strokeWidth={2} aria-hidden="true" />
                                        {words.matchesNone}
                                    </span>
                                ) : (
                                    words.matches(count)
                                )}
                            </li>
                        );
                    })}
                </ul>
            )}
        </div>
    );
}
