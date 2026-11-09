import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Ban as BanIcon, ChevronDown, Undo2 } from 'lucide-react';
import { useId, useState } from 'react';

import { allBans, banPlayer, characters, liftBan, MAX_MINUTES, MAX_REASON } from '../api/bans';
import type { Ban } from '../api/generated/Ban';
import { Button } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { FieldError, inputClass, TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useNow } from '../lib/useNow';
import { useVisit } from '../players/words';
import { useSchedule } from '../schedule/context';
import { minutesOf } from './words';

const LENGTHS = ['hour', 'day', 'threeDays', 'week', 'month', 'permanent', 'other'] as const;
type Length = (typeof LENGTHS)[number];
const PRESET: Record<Exclude<Length, 'other'>, number | null> = {
    hour: 60,
    day: 24 * 60,
    threeDays: 3 * 24 * 60,
    week: 7 * 24 * 60,
    month: 30 * 24 * 60,
    permanent: null,
};
const UNITS = { minutes: 1, hours: 60, days: 24 * 60 } as const;
type Unit = keyof typeof UNITS;

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
    const reasonId = useId();
    const amountId = useId();
    const unitId = useId();
    const identifiersId = useId();
    const [length, setLength] = useState<Length>('day');
    const [amount, setAmount] = useState('2');
    const [unit, setUnit] = useState<Unit>('hours');
    const [reason, setReason] = useState('');
    const [typed, setTyped] = useState('');
    const [tried, setTried] = useState(false);
    const now = useNow(30_000);
    const minutes = length === 'other' ? Math.round(Number(amount) * UNITS[unit]) : PRESET[length];
    const lengthWrong =
        length === 'other' &&
        (!Number.isFinite(minutes) || minutes === null || minutes < 1 || minutes > MAX_MINUTES);
    const reasonWrong = reason.trim() === '' || characters(reason.trim()) > MAX_REASON;
    const identifiers = typed
        .split('\n')
        .map((line) => line.trim())
        .filter((line) => line !== '');
    const identifiersWrong = target.player === null && identifiers.length === 0;
    const ban = useMutation({
        mutationFn: () =>
            banPlayer({
                player: target.player?.id ?? null,
                identifiers,
                reason: reason.trim(),
                minutes,
            }),
        onSuccess: async (made) => {
            await client.invalidateQueries(allBans);
            onBanned(made);
        },
    });
    const submit = () => {
        setTried(true);
        if (!lengthWrong && !reasonWrong && !identifiersWrong && !ban.isPending) {
            ban.mutate();
        }
    };
    const ends = minutes === null || lengthWrong ? null : new Date(now + minutes * 60_000).toISOString();
    const label =
        length === 'permanent'
            ? words.submitPermanent
            : words.submit(
                  length === 'other' ? t.bans.lengthOf(minutes).toLowerCase() : words.lengths[length],
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
            <div className="flex flex-col gap-2">
                <Segmented
                    label={words.length}
                    choices={LENGTHS}
                    value={length}
                    words={words.lengths}
                    onChange={setLength}
                />
                {length === 'other' && (
                    <div className="flex flex-wrap items-end gap-2">
                        <div className="flex flex-col gap-1.5">
                            <label htmlFor={amountId} className="text-ui font-medium text-ink">
                                {words.amount}
                            </label>
                            <input
                                id={amountId}
                                type="number"
                                inputMode="numeric"
                                min={1}
                                className={cn(inputClass, 'w-28')}
                                aria-invalid={(tried && lengthWrong) || undefined}
                                value={amount}
                                onChange={(event) => setAmount(event.target.value)}
                            />
                        </div>
                        <div className="flex flex-col gap-1.5">
                            <label htmlFor={unitId} className="text-ui font-medium text-ink">
                                {words.unit}
                            </label>
                            <div className="relative">
                                <select
                                    id={unitId}
                                    className={cn(inputClass, 'w-36 appearance-none pr-9')}
                                    value={unit}
                                    onChange={(event) => setUnit(event.target.value as Unit)}
                                >
                                    {(Object.keys(UNITS) as Unit[]).map((each) => (
                                        <option key={each} value={each}>
                                            {words.units[each]}
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
                        </div>
                    </div>
                )}
                {tried && lengthWrong ? (
                    <FieldError>{words.outOfRange}</FieldError>
                ) : (
                    <p className="text-small text-ink-3" aria-live="polite">
                        {ends === null ? words.never : words.ends(visit(ends), zone)}
                    </p>
                )}
            </div>
            <div className="flex flex-col gap-1.5">
                <div className="flex items-baseline justify-between gap-3">
                    <label htmlFor={reasonId} className="text-ui font-medium text-ink">
                        {words.reason}
                    </label>
                    <span className="font-mono text-meta text-ink-3">
                        {words.counter(characters(reason.trim()), MAX_REASON)}
                    </span>
                </div>
                <textarea
                    id={reasonId}
                    rows={2}
                    maxLength={MAX_REASON * 2}
                    aria-invalid={(tried && reasonWrong) || undefined}
                    aria-describedby={`${reasonId}-hint`}
                    className={cn(inputClass, 'h-auto min-h-16 resize-y py-2.5 leading-[1.55]')}
                    value={reason}
                    onChange={(event) => setReason(event.target.value)}
                />
                {tried && reasonWrong ? (
                    <FieldError>{words.reasonMissing}</FieldError>
                ) : (
                    <p id={`${reasonId}-hint`} className="text-small text-ink-3">
                        {words.reasonHint}
                    </p>
                )}
            </div>
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
