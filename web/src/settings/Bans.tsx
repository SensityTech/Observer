import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowDown, ArrowRight, ArrowUp, Pencil, Plus, X } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import {
    banTemplatesQuery,
    bansSettingsQuery,
    characters,
    fillMessage,
    MAX_MESSAGE,
    MAX_TEMPLATES,
    MAX_TOKENS,
    saveBansSettings,
} from '../api/bans';
import type { BansSettings as Settings } from '../api/generated/BansSettings';
import type { BanTemplate } from '../api/generated/BanTemplate';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import {
    A_DAY,
    draftMinutes,
    draftOf,
    LengthField,
    type LengthDraft,
    NoSuchLength,
    ReasonField,
    reasonFits,
} from '../bans/fields';
import { Button, buttonVariants } from '../components/Button';
import { Chip } from '../components/Chip';
import { Dialog } from '../components/Dialog';
import { FieldError, inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { SettingsForm, useSection } from './Form';
import { CategoryHead, SettingRow, SettingsGroup } from './Row';

type Outcome = { tone: 'success' | 'danger'; message: string };
const PLACEHOLDERS = ['reason', 'ends', 'id', 'name', 'by'] as const;

/** A template of the draft, with a key of its own: the list is reordered, and two may read alike while typed. */
type Row = BanTemplate & { key: number };
type Draft = { message: string; tokenMatches: string; templates: Row[] };
/** A template being added or edited, in a dialog outside the settings' form. */
type Asking = {
    template: BanTemplate | null;
    others: BanTemplate[];
    onSave: (template: BanTemplate) => void;
};
type Move = 'up' | 'down';

const draftFrom = (stored: Settings): Draft => ({
    message: stored.message,
    tokenMatches: String(stored.tokenMatches),
    templates: stored.templates.map((template, key) => ({ ...template, key })),
});

const sameTemplate = (one: BanTemplate, other: BanTemplate | undefined) =>
    one.reason === other?.reason && one.minutes === other.minutes;

const templatesOf = (rows: Row[]) => (current: Draft) => ({ ...current, templates: rows });

/** What a banned player reads, how much of a ban refuses a player, and the reasons the ban dialog offers. */
export function BansSettings() {
    const { t } = useI18n();
    const me = useMe();
    const settings = useQuery(bansSettingsQuery);
    const [asking, setAsking] = useState<Asking | null>(null);
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
                    <BansForm stored={settings.data} onAsk={setAsking} />
                </SettingsForm>
            )}
            {asking !== null && (
                <TemplateDialog
                    asking={asking}
                    onClose={() => setAsking(null)}
                    onSave={(template) => {
                        asking.onSave(template);
                        setAsking(null);
                    }}
                />
            )}
        </>
    );
}

function BansForm({ stored, onAsk }: { stored: Settings; onAsk: (asking: Asking) => void }) {
    const { t } = useI18n();
    const words = t.settings.bans;
    const me = useMe();
    const client = useQueryClient();
    const canManage = allows(me, Permission.SettingsManage);
    const message = useRef<HTMLTextAreaElement>(null);
    const [draft, setDraft] = useState(() => draftFrom(stored));
    const nextKey = useRef(stored.templates.length);
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
    const templatesChanged =
        draft.templates.length !== stored.templates.length ||
        draft.templates.some((template, index) => !sameTemplate(template, stored.templates[index]));
    const changed = [
        ...(draft.message !== stored.message ? (['refusalMessage'] as const) : []),
        ...(tokens !== stored.tokenMatches ? (['tokenMatches'] as const) : []),
        ...(templatesChanged ? (['banTemplates'] as const) : []),
    ];
    const save = useMutation({
        mutationFn: () =>
            saveBansSettings({
                message: draft.message,
                tokenMatches: tokens,
                templates: draft.templates.map(({ reason, minutes }) => ({ reason, minutes })),
            }),
        onMutate: () => setOutcome(null),
        onSuccess: (saved) => {
            client.setQueryData(bansSettingsQuery.queryKey, saved);
            client.setQueryData(banTemplatesQuery.queryKey, saved.templates);
            setDraft(draftFrom(saved));
            nextKey.current = saved.templates.length;
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
            setDraft(draftFrom(stored));
            nextKey.current = stored.templates.length;
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

    const controls = useRef(new Map<string, HTMLButtonElement>());
    const addButton = useRef<HTMLButtonElement>(null);
    /** Where the keyboard goes once the list changed under it: a control of a row, or the Add button. */
    const focusNext = useRef<string | null>(null);
    const control = (name: string) => (button: HTMLButtonElement | null) => {
        if (button === null) {
            controls.current.delete(name);
        } else {
            controls.current.set(name, button);
        }
    };
    useEffect(() => {
        const name = focusNext.current;
        if (name === null) {
            return;
        }
        focusNext.current = null;
        const target = controls.current.get(name);
        (target === undefined || target.disabled ? addButton.current : target)?.focus();
    });
    const move = (index: number, way: Move) => {
        const to = way === 'up' ? index - 1 : index + 1;
        const rows = [...draft.templates];
        const [row, other] = [rows[index], rows[to]];
        if (row === undefined || other === undefined) {
            return;
        }
        rows[index] = other;
        rows[to] = row;
        // At an end of the list its own arrow goes off: the other one takes the keyboard.
        const atEnd = way === 'up' ? to === 0 : to === rows.length - 1;
        focusNext.current = `${atEnd ? (way === 'up' ? 'down' : 'up') : way} ${row.key}`;
        setDraft(templatesOf(rows));
    };
    const remove = (index: number) => {
        const next = draft.templates[index + 1] ?? draft.templates[index - 1];
        focusNext.current = next === undefined ? 'add' : `remove ${next.key}`;
        setDraft(templatesOf(draft.templates.filter((_, each) => each !== index)));
    };
    const ask = (row: Row | null) =>
        onAsk({
            template: row,
            others: draft.templates.filter((each) => each.key !== row?.key),
            onSave: (template) =>
                setDraft((current) => ({
                    ...current,
                    templates:
                        row === null
                            ? [...current.templates, { ...template, key: nextKey.current++ }]
                            : current.templates.map((each) =>
                                  each.key === row.key ? { ...template, key: row.key } : each,
                              ),
                })),
        });
    const full = draft.templates.length >= MAX_TEMPLATES;

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
                <SettingRow id="banTemplates" changed={canManage && changed.includes('banTemplates')}>
                    {(ids) => (
                        <fieldset
                            aria-labelledby={ids.label}
                            aria-describedby={ids.about}
                            className="flex min-w-0 flex-col gap-3"
                        >
                            {draft.templates.length === 0 ? (
                                <p className="text-small text-ink-3">{words.noTemplates}</p>
                            ) : (
                                <ol
                                    aria-label={words.templates}
                                    className="flex flex-col rounded-box border border-line"
                                >
                                    {draft.templates.map((template, index) => (
                                        <TemplateRow
                                            key={template.key}
                                            template={template}
                                            first={index === 0}
                                            last={index === draft.templates.length - 1}
                                            control={control}
                                            onMove={(way) => move(index, way)}
                                            onEdit={() => ask(template)}
                                            onRemove={() => remove(index)}
                                        />
                                    ))}
                                </ol>
                            )}
                            <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                                <Button
                                    ref={addButton}
                                    icon={Plus}
                                    className="max-md:flex-1"
                                    disabled={full}
                                    onClick={() => ask(null)}
                                >
                                    {words.addTemplate}
                                </Button>
                                {full && (
                                    <p className="text-small text-ink-3">
                                        {words.templatesFull(MAX_TEMPLATES)}
                                    </p>
                                )}
                            </div>
                        </fieldset>
                    )}
                </SettingRow>
            </SettingsGroup>
        </fieldset>
    );
}

/** A template of the list: its reason, its length, and what moves, edits or removes it. */
function TemplateRow({
    template,
    first,
    last,
    control,
    onMove,
    onEdit,
    onRemove,
}: {
    template: Row;
    first: boolean;
    last: boolean;
    control: (name: string) => (button: HTMLButtonElement | null) => void;
    onMove: (way: Move) => void;
    onEdit: () => void;
    onRemove: () => void;
}) {
    const { t } = useI18n();
    const words = t.settings.bans;
    const length = t.bans.lengthOf(template.minutes);
    // Two templates may share a reason: their lengths tell their buttons apart.
    const name = t.bans.dialog.templateOption(template.reason, length);
    const icon = buttonVariants({ size: 'icon-sm', tone: 'ghost' });
    return (
        <li className="flex flex-wrap items-center justify-end gap-x-2 gap-y-1 border-b border-line py-1.5 pr-1.5 pl-3 last:border-b-0">
            <span className="min-w-0 flex-[1_1_8rem] text-ui text-ink [overflow-wrap:anywhere]">
                {template.reason}
            </span>
            <Chip tone={template.minutes === null ? 'danger' : 'neutral'}>{length}</Chip>
            <span className="flex">
                <button
                    ref={control(`up ${template.key}`)}
                    type="button"
                    className={icon}
                    aria-label={words.moveUp(name)}
                    disabled={first}
                    onClick={() => onMove('up')}
                >
                    <ArrowUp size={14} strokeWidth={1.5} aria-hidden="true" />
                </button>
                <button
                    ref={control(`down ${template.key}`)}
                    type="button"
                    className={icon}
                    aria-label={words.moveDown(name)}
                    disabled={last}
                    onClick={() => onMove('down')}
                >
                    <ArrowDown size={14} strokeWidth={1.5} aria-hidden="true" />
                </button>
                <button type="button" className={icon} aria-label={words.editTemplate(name)} onClick={onEdit}>
                    <Pencil size={14} strokeWidth={1.5} aria-hidden="true" />
                </button>
                <button
                    ref={control(`remove ${template.key}`)}
                    type="button"
                    className={icon}
                    aria-label={words.removeTemplate(name)}
                    onClick={onRemove}
                >
                    <X size={14} strokeWidth={1.5} aria-hidden="true" />
                </button>
            </span>
        </li>
    );
}

/** A template's reason and length. What it saves joins the draft: the settings' save bar keeps it. */
function TemplateDialog({
    asking,
    onClose,
    onSave,
}: {
    asking: Asking;
    onClose: () => void;
    onSave: (template: BanTemplate) => void;
}) {
    const { t } = useI18n();
    const words = t.settings.bans.templateDialog;
    const { template, others } = asking;
    const [reason, setReason] = useState(template?.reason ?? '');
    const [length, setLength] = useState<LengthDraft>(template === null ? A_DAY : draftOf(template.minutes));
    const [tried, setTried] = useState(false);
    const minutes = draftMinutes(length);
    const reasonWrong = !reasonFits(reason);
    const listed =
        minutes !== undefined &&
        others.some((each) => sameTemplate({ reason: reason.trim(), minutes }, each));
    return (
        <Dialog
            title={template === null ? words.add : words.edit}
            onClose={onClose}
            closeOnBackdrop={reason === (template?.reason ?? '')}
            onSubmit={() => {
                setTried(true);
                if (minutes !== undefined && !reasonWrong && !listed) {
                    onSave({ reason: reason.trim(), minutes });
                }
            }}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink">
                        {template === null ? words.submitAdd : words.submitEdit}
                    </Button>
                </>
            }
        >
            <ReasonField
                value={reason}
                hint={words.reasonHint}
                error={
                    tried && reasonWrong
                        ? t.bans.dialog.reasonMissing
                        : tried && listed
                          ? words.listed
                          : undefined
                }
                onChange={setReason}
            />
            <div className="flex flex-col gap-2">
                <p className="text-ui font-medium text-ink" aria-hidden="true">
                    {t.bans.dialog.length}
                </p>
                <LengthField
                    label={t.bans.dialog.length}
                    draft={length}
                    invalid={tried && minutes === undefined}
                    onChange={setLength}
                />
                {minutes === undefined && <NoSuchLength tried={tried} />}
            </div>
            <p className="text-small text-ink-3">{words.kept}</p>
        </Dialog>
    );
}
