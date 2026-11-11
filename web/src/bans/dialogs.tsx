import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Ban as BanIcon, ChevronDown, Pencil, Undo2 } from 'lucide-react';
import { useId, useState } from 'react';

import { allBans, banPlayer, banTemplatesQuery, editBan, liftBan } from '../api/bans';
import type { Ban } from '../api/generated/Ban';
import { Button } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { FieldError, inputClass, TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useNow } from '../lib/useNow';
import { useVisit } from '../players/words';
import { useSchedule } from '../schedule/context';
import {
    A_DAY,
    draftMinutes,
    draftOf,
    LengthField,
    type LengthDraft,
    NoSuchLength,
    ReasonField,
    reasonFits,
} from './fields';
import { minutesOf } from './words';

/** Who is banned: a player Observer knows, or identifiers typed by hand. */
export type BanTarget =
    { player: { id: number; name: string; license: string; online: boolean } } | { player: null };

/** Bans a player, or identifiers typed by hand, for a length and a reason the player reads. */
export function BanDialog({
    target,
    onClose,
    onBanned,
}: {
    target: BanTarget;
    onClose: () => void;
    onBanned: (ban: Ban) => void;
}) {
    const { t } = useI18n();
    const words = t.bans.dialog;
    const client = useQueryClient();
    const zone = useSchedule().zone ?? 'UTC';
    const visit = useVisit(zone);
    const templateId = useId();
    const identifiersId = useId();
    const templates = useQuery(banTemplatesQuery).data ?? [];
    const [length, setLength] = useState<LengthDraft>(A_DAY);
    const [reason, setReason] = useState('');
    const [typed, setTyped] = useState('');
    const [tried, setTried] = useState(false);
    const now = useNow(30_000);
    const minutes = draftMinutes(length);
    const reasonWrong = !reasonFits(reason);
    const identifiers = typed
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line !== '');
    const identifiersWrong = target.player === null && identifiers.length === 0;
    // The template the form still matches: a reason or a length corrected since is no longer one.
    const picked = templates.findIndex(
        (template) => template.reason === reason.trim() && template.minutes === minutes,
    );
    const ban = useMutation({
        mutationFn: (lasts: number | null) =>
            banPlayer({
                player: target.player?.id ?? null,
                identifiers,
                reason: reason.trim(),
                minutes: lasts,
            }),
        onSuccess: async (made) => {
            await client.invalidateQueries(allBans);
            onBanned(made);
        },
    });
    const submit = () => {
        setTried(true);
        if (minutes !== undefined && !reasonWrong && !identifiersWrong && !ban.isPending) {
            ban.mutate(minutes);
        }
    };
    const label =
        minutes === null
            ? words.submitPermanent
            : minutes === undefined
              ? words.submitNoLength
              : words.submit(
                    length.length === 'other' || length.length === 'keep'
                        ? t.bans.lengthOf(minutes).toLowerCase()
                        : words.lengths[length.length],
                );

    return (
        <Dialog
            title={target.player === null ? words.titleIdentifiers : words.titlePlayer(target.player.name)}
            onClose={onClose}
            closeOnBackdrop={reason === '' && typed === ''}
            onSubmit={submit}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="danger" icon={BanIcon} pending={ban.isPending}>
                        {label}
                    </Button>
                </>
            }
        >
            {target.player !== null && (
                <p className="-mt-1.5 font-mono text-small text-ink-3 [overflow-wrap:anywhere]">
                    {words.licence(target.player.license, target.player.online)}
                </p>
            )}
            {templates.length > 0 && (
                <div className="flex flex-col gap-1.5">
                    <label htmlFor={templateId} className="text-ui font-medium text-ink">
                        {words.template}
                    </label>
                    <div className="relative">
                        <select
                            id={templateId}
                            className={cn(inputClass, 'appearance-none truncate pr-9')}
                            aria-describedby={`${templateId}-hint`}
                            value={picked === -1 ? '' : String(picked)}
                            onChange={(event) => {
                                const template = templates[Number(event.target.value)];
                                const chosen = event.target.value === '' ? undefined : template;
                                setReason(chosen?.reason ?? '');
                                setLength(chosen === undefined ? A_DAY : draftOf(chosen.minutes));
                            }}
                        >
                            <option value="">{words.noTemplate}</option>
                            {templates.map((template, index) => (
                                <option key={`${template.minutes} ${template.reason}`} value={index}>
                                    {words.templateOption(template.reason, t.bans.lengthOf(template.minutes))}
                                </option>
                            ))}
                        </select>
                        <ChevronDown
                            size={16}
                            strokeWidth={1.5}
                            className="pointer-events-none absolute top-1/2 right-3 -translate-y-1/2 text-ink-3"
                            aria-hidden="true"
                        />
                    </div>
                    <p id={`${templateId}-hint`} className="text-small text-ink-3">
                        {words.templateHint}
                    </p>
                </div>
            )}
            <div className="flex flex-col gap-2">
                <LengthField
                    label={words.length}
                    draft={length}
                    invalid={tried && minutes === undefined}
                    onChange={setLength}
                />
                {minutes === undefined ? (
                    <NoSuchLength tried={tried} />
                ) : (
                    <p className="text-small text-ink-3" aria-live="polite">
                        {minutes === null
                            ? words.never
                            : words.ends(visit(new Date(now + minutes * 60_000).toISOString()), zone)}
                    </p>
                )}
            </div>
            <ReasonField
                value={reason}
                hint={words.reasonHint}
                error={tried && reasonWrong ? words.reasonMissing : undefined}
                onChange={setReason}
            />
            {target.player === null ? (
                <div className="flex flex-col gap-1.5">
                    <label htmlFor={identifiersId} className="text-ui font-medium text-ink">
                        {words.identifiers}
                    </label>
                    <textarea
                        id={identifiersId}
                        rows={4}
                        spellCheck={false}
                        autoCapitalize="none"
                        autoComplete="off"
                        aria-invalid={(tried && identifiersWrong) || undefined}
                        aria-describedby={`${identifiersId}-hint`}
                        className={cn(
                            inputClass,
                            'h-auto min-h-24 resize-y py-2.5 font-mono text-ui leading-[1.55]',
                        )}
                        value={typed}
                        onChange={(event) => setTyped(event.target.value)}
                    />
                    {tried && identifiersWrong ? (
                        <FieldError>{words.identifiersMissing}</FieldError>
                    ) : (
                        <p id={`${identifiersId}-hint`} className="text-small text-ink-3">
                            {words.identifiersHint}
                        </p>
                    )}
                </div>
            ) : (
                <div className="flex flex-col gap-1">
                    <p className="text-ui font-medium text-ink">{words.holds}</p>
                    <p className="text-small text-ink-3">{words.holdsProfile}</p>
                </div>
            )}
            {target.player?.online === true && (
                <Notice tone="warning">{words.online(target.player.name)}</Notice>
            )}
            {ban.isError && <Notice tone="danger">{errorMessage(t, ban.error)}</Notice>}
        </Dialog>
    );
}

/** Changes an active ban's reason, its end, or both. A new length counts from now. */
export function EditDialog({
    ban,
    zone,
    onClose,
    onEdited,
}: {
    ban: Ban;
    /** The zone the page it opens from reads times in. */
    zone: string;
    onClose: () => void;
    onEdited: (ban: Ban) => void;
}) {
    const { t } = useI18n();
    const words = t.bans.editDialog;
    const client = useQueryClient();
    const visit = useVisit(zone);
    const [reason, setReason] = useState(ban.reason);
    const [length, setLength] = useState<LengthDraft>({ ...A_DAY, length: 'keep' });
    const [tried, setTried] = useState(false);
    const now = useNow(30_000);
    const keeps = length.length === 'keep';
    const minutes = draftMinutes(length);
    const lengthWrong = !keeps && minutes === undefined;
    const reasonWrong = !reasonFits(reason);
    const reasonChanges = reason.trim() !== ban.reason;
    const endChanges = !keeps && !(minutes === null && ban.endsAt === null);
    const endsAt =
        minutes === undefined
            ? ban.endsAt
            : minutes === null
              ? null
              : new Date(now + minutes * 60_000).toISOString();
    const nothing = !reasonChanges && !endChanges;
    const edit = useMutation({
        mutationFn: () =>
            editBan(ban.code, {
                reason: reasonChanges ? reason.trim() : null,
                ends:
                    !endChanges || minutes === undefined
                        ? null
                        : minutes === null
                          ? { kind: 'never' }
                          : { kind: 'in', minutes },
            }),
        onSuccess: async (edited) => {
            await client.invalidateQueries(allBans);
            onEdited(edited);
        },
    });
    const submit = () => {
        setTried(true);
        if (!lengthWrong && !reasonWrong && !nothing && !edit.isPending) {
            edit.mutate();
        }
    };
    const before = ban.endsAt === null ? null : visit(ban.endsAt);

    return (
        <Dialog
            title={ban.name === null ? words.titleCode(ban.code) : words.title(ban.name)}
            className="max-w-xl"
            onClose={onClose}
            closeOnBackdrop={!reasonChanges && keeps}
            onSubmit={submit}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" icon={Pencil} pending={edit.isPending}>
                        {words.submit}
                    </Button>
                </>
            }
        >
            <p className="-mt-1.5 text-small text-ink-3">
                {t.bans.profile.madeBy(ban.by, visit(ban.createdAt), ban.code)}
            </p>
            <ReasonField
                value={reason}
                hint={t.bans.dialog.reasonHint}
                error={tried && reasonWrong ? t.bans.dialog.reasonMissing : undefined}
                onChange={setReason}
            />
            <div className="flex flex-col gap-2">
                <p className="text-ui font-medium text-ink" aria-hidden="true">
                    {words.ends}
                </p>
                <LengthField
                    label={words.ends}
                    keep={words.keep}
                    draft={length}
                    invalid={tried && lengthWrong}
                    onChange={setLength}
                />
                {lengthWrong ? (
                    <NoSuchLength tried={tried} />
                ) : (
                    <p className="text-small text-ink-3" aria-live="polite">
                        {!endChanges
                            ? before === null
                                ? words.stillNever
                                : words.stillEnds(before, zone)
                            : endsAt === null
                              ? words.neverNow(before ?? '')
                              : words.endsNow(visit(endsAt), zone, before)}
                    </p>
                )}
            </div>
            {tried && nothing && !lengthWrong && <Notice>{words.unchanged}</Notice>}
            {edit.isError && <Notice tone="danger">{errorMessage(t, edit.error)}</Notice>}
        </Dialog>
    );
}

/** Lifts an active ban, with an optional reason kept beside it. */
export function LiftDialog({
    ban,
    onClose,
    onLifted,
}: {
    ban: Ban;
    onClose: () => void;
    onLifted: () => void;
}) {
    const { t } = useI18n();
    const words = t.bans.liftDialog;
    const client = useQueryClient();
    const [why, setWhy] = useState('');
    const lift = useMutation({
        mutationFn: () => liftBan(ban.code, why),
        onSuccess: async () => {
            await client.invalidateQueries(allBans);
            onLifted();
        },
    });
    return (
        <Dialog
            title={ban.name === null ? words.titleCode(ban.code) : words.title(ban.name)}
            onClose={onClose}
            closeOnBackdrop={why === ''}
            onSubmit={() => {
                if (!lift.isPending) {
                    lift.mutate();
                }
            }}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" icon={Undo2} pending={lift.isPending}>
                        {words.submit}
                    </Button>
                </>
            }
        >
            <p>{words.lead}</p>
            <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-ui">
                <dt className="text-ink-3">{t.bans.code}</dt>
                <dd className="font-mono text-ink">{ban.code}</dd>
                <dt className="text-ink-3">{t.bans.reason}</dt>
                <dd className="text-ink [overflow-wrap:anywhere]">{ban.reason}</dd>
                <dt className="text-ink-3">{t.bans.profile.length}</dt>
                <dd className="text-ink">{t.bans.lengthOf(minutesOf(ban))}</dd>
            </dl>
            <TextField
                label={
                    <>
                        {words.why} <span className="font-normal text-ink-3">({t.common.optional})</span>
                    </>
                }
                hint={words.whyHint}
                value={why}
                maxLength={400}
                onChange={(event) => setWhy(event.target.value)}
            />
            {lift.isError && <Notice tone="danger">{errorMessage(t, lift.error)}</Notice>}
        </Dialog>
    );
}
