import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight } from 'lucide-react';
import { useRef, useState } from 'react';

import {
    bansSettingsQuery,
    characters,
    fillMessage,
    MAX_MESSAGE,
    MAX_TOKENS,
    saveBansSettings,
} from '../api/bans';
import type { BansSettings as Settings } from '../api/generated/BansSettings';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { FieldError, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { SettingsForm, useSection } from './Form';
import { CategoryHead, SettingRow, SettingsGroup } from './Row';

type Outcome = { tone: 'success' | 'danger'; message: string };
const PLACEHOLDERS = ['reason', 'ends', 'id', 'name', 'by'] as const;

/** What a banned player reads, and how much of a ban a player must show to be refused. */
export function BansSettings() {
    const { t } = useI18n();
    const me = useMe();
    const settings = useQuery(bansSettingsQuery);
    return (
        <>
            <CategoryHead
                category="bans"
                aside={
                    allows(me, Permission.BansView) ? (
                        <Link
                            to="/bans"
                            className="inline-flex items-center gap-1.5 text-ui font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                        >
                            {t.settings.bans.openBans}
                            <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                        </Link>
                    ) : undefined
                }
            />
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
                    {t.settings.bans.loadFailed} {errorMessage(t, settings.error)}
                </Notice>
            )}
            {settings.isSuccess && (
                <SettingsForm>
                    <BansForm stored={settings.data} />
                </SettingsForm>
            )}
        </>
    );
}

function BansForm({ stored }: { stored: Settings }) {
    const { t } = useI18n();
    const words = t.settings.bans;
    const me = useMe();
    const client = useQueryClient();
    const canManage = allows(me, Permission.SettingsManage);
    const message = useRef<HTMLTextAreaElement>(null);
    const [draft, setDraft] = useState({
        message: stored.message,
        tokenMatches: String(stored.tokenMatches),
    });
    const [tried, setTried] = useState(false);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const tokens = /^\d+$/.test(draft.tokenMatches.trim()) ? Number(draft.tokenMatches.trim()) : Number.NaN;
    const messageProblem =
        draft.message.trim() === ''
            ? words.messageMissing
            : characters(draft.message.trim()) > MAX_MESSAGE
              ? words.messageTooLong(MAX_MESSAGE)
              : null;
    const tokensProblem =
        Number.isInteger(tokens) && tokens <= MAX_TOKENS ? null : words.tokensOutOfRange(MAX_TOKENS);
    const changed = [
        ...(draft.message !== stored.message ? (['refusalMessage'] as const) : []),
        ...(tokens !== stored.tokenMatches ? (['tokenMatches'] as const) : []),
    ];
    const save = useMutation({
        mutationFn: () => saveBansSettings({ message: draft.message, tokenMatches: tokens }),
        onMutate: () => setOutcome(null),
        onSuccess: (saved) => {
            client.setQueryData(bansSettingsQuery.queryKey, saved);
            setDraft({ message: saved.message, tokenMatches: String(saved.tokenMatches) });
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
            if (messageProblem === null && tokensProblem === null) {
                save.mutate();
            }
        },
        discard: () => {
            setDraft({ message: stored.message, tokenMatches: String(stored.tokenMatches) });
            setTried(false);
            setOutcome({ tone: 'success', message: words.discarded });
        },
    });
    const insert = (key: (typeof PLACEHOLDERS)[number]) => {
        const field = message.current;
        const text = `{${key}}`;
        const start = field?.selectionStart ?? draft.message.length;
        const end = field?.selectionEnd ?? draft.message.length;
        setDraft({ ...draft, message: draft.message.slice(0, start) + text + draft.message.slice(end) });
        requestAnimationFrame(() => {
            field?.focus();
            field?.setSelectionRange(start + text.length, start + text.length);
        });
    };

    return (
        <fieldset disabled={!canManage} className="flex min-w-0 flex-col gap-4">
            {!canManage && <Notice>{words.readOnly}</Notice>}
            {outcome !== null && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
            <SettingsGroup title={t.settings.categories.bans.name}>
                <SettingRow
                    id="refusalMessage"
                    field
                    changed={canManage && changed.includes('refusalMessage')}
                >
                    {(ids) => (
                        <>
                            <textarea
                                ref={message}
                                id={ids.control}
                                rows={Math.min(10, Math.max(4, draft.message.split('\n').length + 1))}
                                aria-invalid={(tried && messageProblem !== null) || undefined}
                                aria-describedby={
                                    tried && messageProblem !== null
                                        ? `${ids.control}-error ${ids.about}`
                                        : ids.about
                                }
                                className={cn(
                                    inputClass,
                                    'h-auto min-h-28 resize-y py-2.5 font-mono text-ui leading-[1.55]',
                                )}
                                value={draft.message}
                                onChange={(event) => setDraft({ ...draft, message: event.target.value })}
                            />
                            {tried && messageProblem !== null && (
                                <FieldError id={`${ids.control}-error`}>{messageProblem}</FieldError>
                            )}
                            <div className="flex flex-wrap items-center gap-1.5">
                                <span className="mr-1 text-small text-ink-3">{words.insert}</span>
                                {PLACEHOLDERS.map((key) => (
                                    <button
                                        key={key}
                                        type="button"
                                        className="inline-flex h-7 items-center rounded-xs border border-dashed border-line-strong px-2 font-mono text-small text-accent-ink hover:bg-hover max-md:h-11"
                                        onClick={() => insert(key)}
                                    >
                                        {`{${key}}`}
                                    </button>
                                ))}
                            </div>
                            <figure className="m-0 flex flex-col gap-1.5 rounded-box border border-line bg-bg-subtle px-3.5 py-3">
                                <figcaption className="text-meta text-ink-3">{words.preview}</figcaption>
                                <p className="text-ui whitespace-pre-line text-ink [overflow-wrap:anywhere]">
                                    {fillMessage(draft.message.trim(), {
                                        reason: words.example.reason,
                                        ends: words.example.ends,
                                        id: words.example.code,
                                        name: words.example.name,
                                        by: words.example.by,
                                    })}
                                </p>
                            </figure>
                        </>
                    )}
                </SettingRow>
                <SettingRow id="tokenMatches" field changed={canManage && changed.includes('tokenMatches')}>
                    {(ids) => (
                        <>
                            <input
                                id={ids.control}
                                type="number"
                                inputMode="numeric"
                                min={0}
                                max={MAX_TOKENS}
                                className={cn(inputClass, 'max-w-32')}
                                aria-invalid={(tried && tokensProblem !== null) || undefined}
                                aria-describedby={
                                    tried && tokensProblem !== null
                                        ? `${ids.control}-error ${ids.about}`
                                        : ids.about
                                }
                                value={draft.tokenMatches}
                                onChange={(event) => setDraft({ ...draft, tokenMatches: event.target.value })}
                            />
                            {tried && tokensProblem !== null && (
                                <FieldError id={`${ids.control}-error`}>{tokensProblem}</FieldError>
                            )}
                        </>
                    )}
                </SettingRow>
            </SettingsGroup>
        </fieldset>
    );
}
