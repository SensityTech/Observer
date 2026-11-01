import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Settings2 } from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';

import { ApiError } from '../api/client';
import type { Differences } from '../api/generated/Differences';
import type { EscrowPolicy } from '../api/generated/EscrowPolicy';
import type { ToolMode } from '../api/generated/ToolMode';
import type { ToolSettings } from '../api/generated/ToolSettings';
import type { ToolView } from '../api/generated/ToolView';
import { saveArchetypeSettings, toolsQuery } from '../api/tools';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { FieldError, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

const MODES: ToolMode[] = ['off', 'report', 'fix'];
const DIFFERENCES: Differences[] = ['identical', 'keepLatest'];
const ESCROW: EscrowPolicy[] = ['skip', 'allow'];

/** What Observer accepts of an excluded name, and how many of them. */
const MAX_NAME = 255;
const MAX_EXCLUDED = 1000;

type Draft = { atStart: ToolMode; differences: Differences; escrowed: EscrowPolicy; excluded: string };
type Shown = { settings: ToolSettings; version: number | null };
type Outcome = { tone: 'success' | 'warning' | 'danger'; message: string };

const toDraft = (settings: ToolSettings): Draft => ({ ...settings, excluded: settings.excluded.join('\n') });

/** The names typed, one a line, each once, in the order typed. */
const names = (text: string) => [
    ...new Set(
        text
            .split('\n')
            .map((line) => line.trim())
            .filter((line) => line !== ''),
    ),
];

const fits = (excluded: string[]) =>
    excluded.length <= MAX_EXCLUDED &&
    excluded.every((name) => new TextEncoder().encode(name).length <= MAX_NAME && !/[/\\]/.test(name));

const countChanges = (saved: ToolSettings, draft: Draft) =>
    [
        draft.atStart !== saved.atStart,
        draft.differences !== saved.differences,
        draft.escrowed !== saved.escrowed,
        names(draft.excluded).join('\n') !== saved.excluded.join('\n'),
    ].filter(Boolean).length;

/** The duplicate archetypes tool's settings, saved over their version with the password asked again. */
export function SettingsCard({ view }: { view: ToolView }) {
    const { t } = useI18n();
    const words = t.tools.settings;
    const id = useId();
    const queryClient = useQueryClient();
    // What the draft was started from: the settings as stored then, and the version a save names.
    const [shown, setShown] = useState<Shown>({ settings: view.settings, version: view.version });
    const [draft, setDraft] = useState(() => toDraft(view.settings));
    const [problem, setProblem] = useState<string | null>(null);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const notice = useRef<HTMLDivElement>(null);
    const changes = countChanges(shown.settings, draft);
    const stale = view.version !== shown.version;

    const startFrom = (stored: Shown) => {
        setShown(stored);
        setDraft(toDraft(stored.settings));
    };

    // Saved elsewhere since: with nothing typed yet, show what is stored now rather than what it
    // replaced, which a save would quietly bring back.
    if (stale && changes === 0) {
        startFrom({ settings: view.settings, version: view.version });
    }

    const save = useMutation({
        mutationFn: (settings: ToolSettings) => saveArchetypeSettings({ settings, version: shown.version }),
        onMutate: () => {
            setOutcome(null);
            setProblem(null);
        },
        onSuccess: (next) => {
            queryClient.setQueryData(toolsQuery.queryKey, next);
            const saved = next.tools.find((tool) => tool.tool === view.tool);
            if (saved !== undefined) {
                startFrom({ settings: saved.settings, version: saved.version });
            }
            setOutcome({ tone: 'success', message: words.saved });
        },
        onError: (error) => {
            const conflict = error instanceof ApiError && error.code === 'conflict';
            if (conflict) {
                void queryClient.invalidateQueries({ queryKey: toolsQuery.queryKey });
            }
            if (error instanceof ApiError && error.code === 'invalid_request') {
                setProblem(words.excludedInvalid);
            }
            setOutcome({
                tone: conflict ? 'warning' : 'danger',
                message: conflict ? words.changedElsewhere : errorMessage(t, error),
            });
        },
    });

    // Saving or discarding takes the save bar away, and with it the button the keyboard was on:
    // it moves to the notice saying what happened rather than back to the top of the document.
    useEffect(() => {
        if (outcome !== null && document.activeElement === document.body) {
            notice.current?.focus();
        }
    }, [outcome]);

    const edit = (change: Partial<Draft>) => {
        setDraft((current) => ({ ...current, ...change }));
        setOutcome(null);
    };

    const submit = () => {
        const excluded = names(draft.excluded);
        if (!fits(excluded)) {
            setOutcome(null);
            setProblem(words.excludedInvalid);
            return;
        }
        save.mutate({
            atStart: draft.atStart,
            differences: draft.differences,
            escrowed: draft.escrowed,
            excluded,
        });
    };

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
            {stale && outcome === null && <Notice tone="warning">{words.changedElsewhere}</Notice>}
            {outcome !== null && (
                <Notice ref={notice} tabIndex={-1} tone={outcome.tone}>
                    {outcome.message}
                </Notice>
            )}
            <Card id={`${id}-title`} icon={Settings2} title={words.title} meta={words.meta}>
                <div className="flex max-w-3xl min-w-0 flex-col gap-5">
                    <Choice
                        legend={words.atStart}
                        hint={words.atStartHint}
                        name={`${id}-at-start`}
                        choices={MODES}
                        words={words.atStartModes}
                        value={draft.atStart}
                        onChange={(atStart) => edit({ atStart })}
                    />
                    <Choice
                        legend={words.differences}
                        hint={words.differencesHint}
                        name={`${id}-differences`}
                        choices={DIFFERENCES}
                        words={words.differencesModes}
                        value={draft.differences}
                        onChange={(differences) => edit({ differences })}
                    />
                    <Choice
                        legend={words.escrowed}
                        hint={words.escrowedHint}
                        name={`${id}-escrowed`}
                        choices={ESCROW}
                        words={words.escrowedModes}
                        value={draft.escrowed}
                        onChange={(escrowed) => edit({ escrowed })}
                    />
                    <div className="flex min-w-0 flex-col gap-1.5">
                        <label htmlFor={`${id}-excluded`} className="text-ui font-medium text-ink">
                            {words.excluded}
                        </label>
                        <textarea
                            id={`${id}-excluded`}
                            rows={Math.min(8, Math.max(3, draft.excluded.split('\n').length + 1))}
                            spellCheck={false}
                            autoCapitalize="none"
                            autoComplete="off"
                            aria-invalid={problem !== null || undefined}
                            aria-describedby={
                                problem === null
                                    ? `${id}-excluded-hint`
                                    : `${id}-excluded-error ${id}-excluded-hint`
                            }
                            className={cn(
                                inputClass,
                                'h-auto min-h-24 resize-y py-2.5 font-mono text-ui leading-[1.55]',
                            )}
                            value={draft.excluded}
                            onChange={(event) => {
                                edit({ excluded: event.target.value });
                                setProblem(null);
                            }}
                        />
                        {problem !== null && <FieldError id={`${id}-excluded-error`}>{problem}</FieldError>}
                        <p id={`${id}-excluded-hint`} className="text-small text-ink-3">
                            {words.excludedHint}
                        </p>
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
                            startFrom({ settings: view.settings, version: view.version });
                            setProblem(null);
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

/** A short exclusive choice of radio buttons, its legend above and what each choice does below. */
function Choice<T extends string>({
    legend,
    hint,
    name,
    choices,
    words,
    value,
    onChange,
}: {
    legend: string;
    hint: string;
    name: string;
    choices: T[];
    words: Record<T, string>;
    value: T;
    onChange: (value: T) => void;
}) {
    const hintId = `${name}-hint`;
    return (
        <fieldset className="flex min-w-0 flex-col gap-1.5" aria-describedby={hintId}>
            <legend className="mb-1.5 text-ui font-medium text-ink">{legend}</legend>
            <div className="inline-flex max-w-full flex-wrap gap-0.5 self-start rounded-control border border-line bg-surface-2 p-0.75">
                {choices.map((choice) => (
                    <label
                        key={choice}
                        className="inline-flex h-6.5 cursor-pointer items-center justify-center rounded-[7px] px-2.5 text-small font-medium text-ink-3 transition-colors hover:text-ink has-checked:bg-raise has-checked:text-ink has-checked:shadow-[var(--elev-sm),0_0_0_1px_var(--line)] has-focus-visible:outline-2 has-focus-visible:outline-offset-1 has-focus-visible:outline-accent max-md:h-11 max-md:min-w-11 max-md:px-3.5"
                    >
                        <input
                            type="radio"
                            name={name}
                            value={choice}
                            className="sr-only"
                            checked={value === choice}
                            onChange={() => onChange(choice)}
                        />
                        {words[choice]}
                    </label>
                ))}
            </div>
            <p id={hintId} className="text-small text-ink-3">
                {hint}
            </p>
        </fieldset>
    );
}
