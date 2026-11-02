import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { ScanSearch, TriangleAlert } from 'lucide-react';
import { useState } from 'react';

import { changesSettingsQuery, saveChangesSettings, verifyFiles } from '../api/changes';
import { ApiError } from '../api/client';
import type { ChangesSettings } from '../api/generated/ChangesSettings';
import type { ChangesSettingsView } from '../api/generated/ChangesSettingsView';
import { Button } from '../components/Button';
import { FieldError, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { Switch } from '../components/Switch';
import { GB, MB, MIB } from '../changes/units';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { stringParam } from '../i18n/params';
import { cn } from '../lib/cn';
import type { SettingId } from './catalog';
import { useSection } from './Form';
import { GroupNotice, SettingRow, SettingsGroup, type RowIds } from './Row';

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

/** The setting each field of the draft is shown as. */
const SETTING_OF = {
    enabled: 'changesEnabled',
    extensions: 'extensions',
    excluded: 'excluded',
    hidden: 'hidden',
    maxText: 'maxText',
    budget: 'budget',
    retentionDays: 'retentionDays',
    hashRate: 'hashRate',
    verifyDays: 'verifyDays',
    startEvent: 'startEvent',
} satisfies Record<keyof Draft, SettingId>;

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

/** The owner's settings of file changes, saved with the password asked again. */
export function ChangesData() {
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
    return <ChangesForm view={settings.data} />;
}

function ChangesForm({ view }: { view: ChangesSettingsView }) {
    const { t, format } = useI18n();
    const words = t.changes.settings;
    const queryClient = useQueryClient();
    const [draft, setDraft] = useState(() => toDraft(view.settings));
    // What is wrong with each field, by its name in the settings.
    const [problems, setProblems] = useState<Partial<Record<string, string>>>({});
    const [outcome, setOutcome] = useState<{ tone: 'success' | 'danger'; message: string } | null>(null);
    const saved = toDraft(view.settings);
    const changed = (Object.keys(draft) as (keyof Draft)[]).filter((key) => draft[key] !== saved[key]);

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

    useSection({
        changed: changed.map((key) => SETTING_OF[key]),
        pending: save.isPending,
        save: () => {
            const settings = fromDraft(draft, view.settings);
            if (Array.isArray(settings)) {
                setOutcome(null);
                setProblems(Object.fromEntries(settings.map((field) => [field, words.outOfRange])));
                return;
            }
            save.mutate(settings);
        },
        discard: () => {
            setDraft(saved);
            setProblems({});
            setOutcome(null);
        },
    });

    const described = (ids: RowIds, field: string) =>
        problems[field] === undefined ? ids.about : `${ids.control}-error ${ids.about}`;
    const number = (field: NumberField, below?: string) => (
        <SettingRow id={field} field changed={changed.includes(field)}>
            {(ids) => (
                <>
                    <input
                        id={ids.control}
                        type="number"
                        inputMode="decimal"
                        min={NUMBERS[field].min}
                        max={NUMBERS[field].max}
                        className={cn(inputClass, 'max-w-40')}
                        aria-invalid={problems[field] !== undefined || undefined}
                        aria-describedby={described(ids, field)}
                        value={draft[field]}
                        onChange={(event) => setDraft({ ...draft, [field]: event.target.value })}
                    />
                    {problems[field] !== undefined && (
                        <FieldError id={`${ids.control}-error`}>{problems[field]}</FieldError>
                    )}
                    {below !== undefined && <p className="text-small text-ink-3">{below}</p>}
                </>
            )}
        </SettingRow>
    );
    const globs = (field: 'excluded' | 'hidden', matches: number[]) => (
        <SettingRow id={field} field changed={changed.includes(field)}>
            {(ids) => (
                <>
                    <textarea
                        id={ids.control}
                        rows={Math.min(8, Math.max(3, draft[field].split('\n').length + 1))}
                        spellCheck={false}
                        autoCapitalize="none"
                        aria-invalid={problems[field] !== undefined || undefined}
                        aria-describedby={described(ids, field)}
                        className={cn(
                            inputClass,
                            'h-auto min-h-24 resize-y py-2.5 font-mono text-ui leading-[1.55]',
                        )}
                        value={draft[field]}
                        onChange={(event) => setDraft({ ...draft, [field]: event.target.value })}
                    />
                    {problems[field] !== undefined && (
                        <FieldError id={`${ids.control}-error`}>{problems[field]}</FieldError>
                    )}
                    {view.settings[field].length > 0 && (
                        <ul className="flex flex-col gap-1">
                            {view.settings[field].map((glob, index) => {
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
                </>
            )}
        </SettingRow>
    );

    return (
        <SettingsGroup
            title={t.settings.data.changes}
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
            {(outcome !== null || verify.isSuccess || verify.isError) && (
                <GroupNotice>
                    {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
                    {verify.isSuccess && <Notice tone="success">{words.verifyStarted}</Notice>}
                    {verify.isError && <Notice tone="danger">{errorMessage(t, verify.error)}</Notice>}
                </GroupNotice>
            )}
            {(['enabled', 'startEvent'] as const).map((field) => (
                <SettingRow key={field} id={SETTING_OF[field]} field changed={changed.includes(field)}>
                    {(ids) => (
                        <Switch
                            id={ids.control}
                            aria-describedby={ids.about}
                            checked={draft[field]}
                            onChange={(event) => setDraft({ ...draft, [field]: event.target.checked })}
                        />
                    )}
                </SettingRow>
            ))}
            <SettingRow id="extensions" field changed={changed.includes('extensions')}>
                {(ids) => (
                    <>
                        <input
                            id={ids.control}
                            className={cn(inputClass, 'font-mono text-ui')}
                            spellCheck={false}
                            autoCapitalize="none"
                            aria-invalid={problems.extensions !== undefined || undefined}
                            aria-describedby={described(ids, 'extensions')}
                            value={draft.extensions}
                            onChange={(event) => setDraft({ ...draft, extensions: event.target.value })}
                        />
                        {problems.extensions !== undefined && (
                            <FieldError id={`${ids.control}-error`}>{problems.extensions}</FieldError>
                        )}
                    </>
                )}
            </SettingRow>
            {globs('excluded', view.excludedMatches)}
            {globs('hidden', view.hiddenMatches)}
            {number('maxText')}
            {number('budget', words.stored(format.bytes(view.stored)))}
            {number('retentionDays')}
            {number('hashRate')}
            {number('verifyDays')}
        </SettingsGroup>
    );
}
