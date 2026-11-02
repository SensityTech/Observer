import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { ApiError } from '../api/client';
import type { Differences } from '../api/generated/Differences';
import type { EscrowPolicy } from '../api/generated/EscrowPolicy';
import type { ToolMode } from '../api/generated/ToolMode';
import type { ToolSettings as Settings } from '../api/generated/ToolSettings';
import type { ToolView } from '../api/generated/ToolView';
import { saveArchetypeSettings, toolsQuery } from '../api/tools';
import { FieldError, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import type { SettingId } from './catalog';
import { keyboardLeft, SettingsForm, useSection } from './Form';
import { CategoryHead, Segmented, SettingRow, SettingsGroup } from './Row';

const MODES: ToolMode[] = ['off', 'report', 'fix'];
const DIFFERENCES: Differences[] = ['identical', 'keepLatest'];
const ESCROW: EscrowPolicy[] = ['skip', 'allow'];

/** What Observer accepts of an excluded name, and how many of them. */
const MAX_NAME = 255;
const MAX_EXCLUDED = 1000;

type Draft = { atStart: ToolMode; differences: Differences; escrowed: EscrowPolicy; excluded: string };
type Shown = { settings: Settings; version: number | null };
type Outcome = { tone: 'success' | 'warning' | 'danger'; message: string };

/** The setting each stored field is shown as. */
const SETTING_OF = {
    atStart: 'toolAtStart',
    differences: 'toolDifferences',
    escrowed: 'toolEscrowed',
    excluded: 'toolExcluded',
} satisfies Record<keyof Settings, SettingId>;

const toDraft = (settings: Settings): Draft => ({ ...settings, excluded: settings.excluded.join('\n') });

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

const changedSettings = (saved: Settings, draft: Draft) =>
    (Object.keys(SETTING_OF) as (keyof Settings)[]).filter((key) =>
        key === 'excluded'
            ? names(draft.excluded).join('\n') !== saved.excluded.join('\n')
            : draft[key] !== saved[key],
    );

/** What the server tools do on their own, the owner's: what sat at the bottom of the Tools page. */
export function ToolSettings() {
    const { t } = useI18n();
    const tools = useQuery(toolsQuery);
    const archetypes = tools.data?.tools.find((tool) => tool.tool === 'duplicateArchetypes');

    return (
        <>
            <CategoryHead
                category="tools"
                aside={
                    <Link
                        to="/tools"
                        className="inline-flex items-center gap-1.5 text-ui font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                    >
                        {t.settings.tools.openTools}
                        <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                    </Link>
                }
            />
            {tools.isPending && (
                <div className="card flex flex-col gap-3 p-5">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-3/4" />
                    <span className="skeleton w-1/2" />
                </div>
            )}
            {/* A failed refetch keeps the data it had, and the settings typed in it. */}
            {tools.isError && tools.data === undefined && (
                <Notice tone="danger">
                    {t.tools.loadFailed} {errorMessage(t, tools.error)}
                </Notice>
            )}
            {archetypes !== undefined && (
                <SettingsForm>
                    <ArchetypeSettings view={archetypes} />
                </SettingsForm>
            )}
        </>
    );
}

/** The duplicate archetypes tool's settings, saved over their version with the password asked again. */
function ArchetypeSettings({ view }: { view: ToolView }) {
    const { t } = useI18n();
    const words = t.tools.settings;
    const queryClient = useQueryClient();
    // What the draft was started from: the settings as stored then, and the version a save names.
    const [shown, setShown] = useState<Shown>({ settings: view.settings, version: view.version });
    const [draft, setDraft] = useState(() => toDraft(view.settings));
    const [problem, setProblem] = useState<string | null>(null);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const notice = useRef<HTMLDivElement>(null);
    const changed = changedSettings(shown.settings, draft);
    const stale = view.version !== shown.version;

    const startFrom = (stored: Shown) => {
        setShown(stored);
        setDraft(toDraft(stored.settings));
    };

    // Saved elsewhere since: with nothing typed yet, show what is stored now rather than what it
    // replaced, which a save would quietly bring back.
    if (stale && changed.length === 0) {
        startFrom({ settings: view.settings, version: view.version });
    }

    const save = useMutation({
        mutationFn: (settings: Settings) => saveArchetypeSettings({ settings, version: shown.version }),
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

    useSection({
        changed: changed.map((key) => SETTING_OF[key]),
        pending: save.isPending,
        save: () => {
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
        },
        discard: () => {
            startFrom({ settings: view.settings, version: view.version });
            setProblem(null);
            setOutcome(null);
        },
    });

    // Saving or discarding takes the save bar away, and with it the button the keyboard was on:
    // it moves to the notice saying what happened rather than back to the top of the document.
    useEffect(() => {
        if (outcome !== null && keyboardLeft()) {
            notice.current?.focus();
        }
    }, [outcome]);

    const edit = (change: Partial<Draft>) => {
        setDraft((current) => ({ ...current, ...change }));
        setOutcome(null);
    };

    return (
        <>
            {stale && outcome === null && <Notice tone="warning">{words.changedElsewhere}</Notice>}
            {outcome !== null && (
                <Notice ref={notice} tabIndex={-1} tone={outcome.tone}>
                    {outcome.message}
                </Notice>
            )}
            <SettingsGroup title={t.tools.archetypes.title}>
                <SettingRow id="toolAtStart" changed={changed.includes('atStart')}>
                    {(ids) => (
                        <Segmented
                            ids={ids}
                            name={`${ids.control}-at-start`}
                            choices={MODES}
                            words={words.atStartModes}
                            value={draft.atStart}
                            onChange={(atStart) => edit({ atStart })}
                        />
                    )}
                </SettingRow>
                <SettingRow id="toolDifferences" changed={changed.includes('differences')}>
                    {(ids) => (
                        <Segmented
                            ids={ids}
                            name={`${ids.control}-differences`}
                            choices={DIFFERENCES}
                            words={words.differencesModes}
                            value={draft.differences}
                            onChange={(differences) => edit({ differences })}
                        />
                    )}
                </SettingRow>
                <SettingRow id="toolEscrowed" changed={changed.includes('escrowed')}>
                    {(ids) => (
                        <Segmented
                            ids={ids}
                            name={`${ids.control}-escrowed`}
                            choices={ESCROW}
                            words={words.escrowedModes}
                            value={draft.escrowed}
                            onChange={(escrowed) => edit({ escrowed })}
                        />
                    )}
                </SettingRow>
                <SettingRow id="toolExcluded" field changed={changed.includes('excluded')}>
                    {(ids) => (
                        <>
                            <textarea
                                id={ids.control}
                                rows={Math.min(8, Math.max(3, draft.excluded.split('\n').length + 1))}
                                spellCheck={false}
                                autoCapitalize="none"
                                autoComplete="off"
                                aria-invalid={problem !== null || undefined}
                                aria-describedby={
                                    problem === null ? ids.about : `${ids.control}-error ${ids.about}`
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
                            {problem !== null && (
                                <FieldError id={`${ids.control}-error`}>{problem}</FieldError>
                            )}
                        </>
                    )}
                </SettingRow>
            </SettingsGroup>
        </>
    );
}
