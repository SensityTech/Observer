import { useMutation } from '@tanstack/react-query';
import {
    CalendarClock,
    CalendarX2,
    Clock,
    History,
    Plus,
    RotateCw,
    SkipForward,
    Undo2,
    X,
    type LucideIcon,
} from 'lucide-react';
import {
    useEffect,
    useId,
    useRef,
    useState,
    type ComponentProps,
    type FormEvent,
    type ReactNode,
} from 'react';

import { ApiError } from '../api/client';
import { Permission } from '../api/generated/Permission';
import type { ScheduleField } from '../api/generated/ScheduleField';
import type { ScheduleIssue } from '../api/generated/ScheduleIssue';
import type { ScheduleMode } from '../api/generated/ScheduleMode';
import type { ScheduleSettings } from '../api/generated/ScheduleSettings';
import type { ScheduleStatus } from '../api/generated/ScheduleStatus';
import type { ServerStatus } from '../api/generated/ServerStatus';
import {
    cancelOnce,
    countChanges,
    DEFAULT_WARNINGS,
    delayNext,
    restartOnce,
    restoreSkipped,
    saveSchedule,
    scheduleIssues,
    skipNext,
} from '../api/schedule';
import { allows, useMe } from '../auth/me';
import { Button, buttonVariants } from '../components/Button';
import { Chip, chipVariants } from '../components/Chip';
import { Dialog } from '../components/Dialog';
import { FieldError, TextField, inputClass } from '../components/Field';
import { Menu } from '../components/Menu';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { formatClock, formatDay, knownZone, zoneOffset } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import { cn } from '../lib/cn';
import { useNow } from '../lib/useNow';
import { useSchedule } from '../schedule/context';
import { EVERY_HOURS, fillTimes, minuteOf, sortTimes } from '../schedule/times';
import { useServerStatus } from '../server/context';
import { isRunning } from '../server/state';

const MODES: ScheduleMode[] = ['fixedTimes', 'interval'];
const DELAYS = [15, 30, 60, 120];
const ONCE_PRESETS = [5, 15, 30];
const ONCE_MAX_MINUTES = 1440;

type Outcome = { tone: 'neutral' | 'success' | 'warning' | 'danger'; message: string };

/** A schedule time in the schedule's zone, as every screen shows it. */
const useZoned = (zone: string) => {
    const { locale } = useI18n();
    return {
        clock: (instant: string | number) => formatClock(locale, instant, zone),
        day: (instant: string | number) => formatDay(locale, instant, zone),
        offset: (instant: string | number) => zoneOffset(locale, zone, instant),
    };
};

export function SchedulePage() {
    const { t } = useI18n();
    const link = useLinkState();
    const { schedule: live, refused } = useSchedule();
    const { status: server } = useServerStatus();
    const now = useNow();
    // While the link is down the page keeps the last status it had, so that the settings form,
    // and what is typed in it, stays.
    const [last, setLast] = useState(live);
    if (live !== null && live !== last) {
        setLast(live);
    }
    const schedule = live ?? (refused ? null : last);

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.schedule.title} <span className="gloss">{t.schedule.gloss}</span>
                </h1>
                {schedule !== null && <ZoneNote zone={schedule.zone} now={now} />}
            </div>
            {schedule !== null && live === null && <Notice tone="warning">{t.schedule.outOfDate}</Notice>}
            {schedule === null ? (
                <section className="card">
                    <div className="card-body flex flex-col gap-3" aria-busy={link !== 'offline'}>
                        <span className="sr-only">
                            {link === 'live' || link === 'connecting'
                                ? t.schedule.loading
                                : t.schedule.unknown}
                        </span>
                        {link === 'live' || link === 'connecting' ? (
                            <>
                                <span className="skeleton w-1/3" />
                                <span className="skeleton w-1/2" />
                            </>
                        ) : (
                            <p className="text-ink-3">{t.schedule.unknown}</p>
                        )}
                    </div>
                </section>
            ) : (
                <>
                    <NextRestart schedule={schedule} server={server} now={now} />
                    <Upcoming schedule={schedule} server={server} now={now} />
                    <SettingsForm schedule={schedule} />
                </>
            )}
        </div>
    );
}

function ZoneNote({ zone, now }: { zone: string; now: number }) {
    const { t } = useI18n();
    const { offset } = useZoned(zone);
    const known = knownZone(zone) === zone;
    return (
        <span className="inline-flex items-center gap-1.5 text-ui text-ink-3">
            <Clock size={14} strokeWidth={1.5} aria-hidden="true" />
            {known ? t.schedule.zone(zone, offset(now)) : t.schedule.zoneUnknown(zone)}
        </span>
    );
}

function Card({
    id,
    icon: Icon,
    title,
    children,
}: {
    id: string;
    icon: LucideIcon;
    title: string;
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
            </div>
            <div className="p-5 max-md:p-4">{children}</div>
        </section>
    );
}

/** A control the account cannot use, or that does nothing now, stays visible and says why when pressed. */
function Action({
    label,
    icon,
    reason,
    pending,
    onPress,
    onBlocked,
}: {
    label: string;
    icon: LucideIcon;
    reason: string | null;
    pending?: boolean;
    onPress: () => void;
    onBlocked: (label: string, reason: string) => void;
}) {
    return (
        <Button
            icon={icon}
            aria-disabled={reason !== null || undefined}
            title={reason ?? undefined}
            pending={pending}
            className={cn(
                'max-md:flex-1',
                reason !== null && 'cursor-not-allowed opacity-45 shadow-none active:translate-y-0',
            )}
            onClick={() => (reason === null ? onPress() : onBlocked(label, reason))}
        >
            {label}
        </Button>
    );
}

function NextRestart({
    schedule,
    server,
    now,
}: {
    schedule: ScheduleStatus;
    server: ServerStatus | null;
    now: number;
}) {
    const { t, format } = useI18n();
    const me = useMe();
    const zoned = useZoned(schedule.zone);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const [planning, setPlanning] = useState(false);
    const canPlan = allows(me, Permission.ScheduleManage);
    const canRestart = allows(me, Permission.ServerControl);
    const running = server !== null && server.state !== 'stopped' && server.state !== 'crashLoop';
    const next = schedule.next;
    const skipped = schedule.upcoming.find((occurrence) => occurrence.skipped?.kind === 'manual');

    const action = useMutation({
        mutationFn: ({
            call,
        }: {
            label: string;
            call: () => Promise<ScheduleStatus>;
            done: (status: ScheduleStatus) => string;
        }) => call(),
        onMutate: () => setOutcome(null),
        onSuccess: (status, { done }) => setOutcome({ tone: 'success', message: done(status) }),
        onError: (error, { label }) =>
            setOutcome({ tone: 'danger', message: t.schedule.failed(label, errorMessage(t, error)) }),
    });
    const blocked = (label: string, reason: string) =>
        setOutcome({ tone: 'warning', message: t.schedule.unavailable(label, reason) });
    const pending = (label: string) => action.isPending && action.variables?.label === label;
    const planReason = canPlan ? null : t.schedule.notPermittedPlan;
    const skipReason =
        planReason ??
        (skipped === undefined ? null : t.schedule.oneSkip(zoned.clock(skipped.delayed?.to ?? skipped.at)));
    const onceReason = !canRestart
        ? t.schedule.notPermittedOnce
        : !running
          ? t.schedule.notRunning
          : server.state === 'stopping'
            ? t.schedule.stoppingOnce
            : null;

    const skip = () =>
        action.mutate({
            label: t.schedule.skip,
            call: skipNext,
            done: (status) => {
                const found = status.upcoming.find((each) => each.skipped?.kind === 'manual');
                return found === undefined
                    ? t.schedule.fromSchedule
                    : t.schedule.skipped(zoned.clock(found.delayed?.to ?? found.at));
            },
        });
    const delay = (minutes: number) =>
        action.mutate({
            label: t.schedule.delay,
            call: () => delayNext(minutes),
            done: (status) => t.schedule.delayed(status.next === null ? '' : zoned.clock(status.next.at)),
        });

    let summary;
    if (next === null) {
        summary = (
            <div className="flex flex-col items-center gap-3 rounded-card border border-dashed border-line-strong bg-bg-subtle px-6 py-10 text-center dot-grid">
                <span className="grid size-12 place-items-center rounded-box border border-line-strong bg-surface text-ink-2 shadow-sm">
                    <CalendarX2 size={20} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <p className="text-[16px] font-semibold text-ink">{t.schedule.emptyTitle}</p>
                <p className="max-w-empty text-body text-ink-3">
                    {schedule.settings.enabled &&
                    schedule.settings.mode === 'interval' &&
                    !(server !== null && isRunning(server.state))
                        ? t.schedule.countsFromStart
                        : t.schedule.emptyBody}
                </p>
            </div>
        );
    } else {
        const left = format.duration(Math.max(0, Date.parse(next.at) - now) / 1000);
        const source =
            next.source === 'oneOff'
                ? t.schedule.oneOff(next.by ?? '')
                : next.delayedFrom !== null
                  ? t.schedule.delayedFrom(zoned.clock(next.delayedFrom), next.by ?? '')
                  : t.schedule.fromSchedule;
        summary = (
            <div className="flex flex-col gap-1">
                <p className="font-mono text-tile text-ink tabular-nums">{zoned.clock(next.at)}</p>
                <p className="text-body text-ink-2">{t.schedule.when(zoned.day(next.at), left)}</p>
                <p className="text-small text-ink-3">{source}</p>
            </div>
        );
    }

    return (
        <Card id="schedule-next" icon={CalendarClock} title={t.schedule.nextTitle}>
            <div className="flex flex-col gap-4">
                {summary}
                {server?.state === 'stopped' && (schedule.settings.enabled || schedule.oneOff !== null) && (
                    <Notice>{t.schedule.stopped}</Notice>
                )}
                {outcome && (
                    <Notice tone={outcome.tone}>
                        <div className="flex items-start gap-3">
                            <p className="flex-1">{outcome.message}</p>
                            <button
                                type="button"
                                className="-m-1 rounded-sm p-1"
                                aria-label={t.schedule.dismiss}
                                onClick={() => setOutcome(null)}
                            >
                                <X size={14} strokeWidth={1.5} aria-hidden="true" />
                            </button>
                        </div>
                    </Notice>
                )}
                <fieldset className="flex min-w-0 flex-wrap gap-2">
                    <legend className="sr-only">{t.schedule.actions}</legend>
                    {schedule.settings.enabled && (
                        <Action
                            label={t.schedule.skip}
                            icon={SkipForward}
                            reason={skipReason}
                            pending={pending(t.schedule.skip)}
                            onPress={skip}
                            onBlocked={blocked}
                        />
                    )}
                    {next?.source === 'schedule' &&
                        (planReason === null ? (
                            <Menu
                                label={t.schedule.delay}
                                placement="below-end"
                                buttonClassName={cn(buttonVariants({ tone: 'secondary' }), 'max-md:flex-1')}
                                items={DELAYS.map((minutes) => ({
                                    label: t.schedule.delayBy(format.duration(minutes * 60)),
                                    icon: History,
                                    onSelect: () => delay(minutes),
                                }))}
                            >
                                <History size={16} strokeWidth={1.5} aria-hidden="true" />
                                {t.schedule.delay}
                            </Menu>
                        ) : (
                            <Action
                                label={t.schedule.delay}
                                icon={History}
                                reason={planReason}
                                onPress={() => {}}
                                onBlocked={blocked}
                            />
                        ))}
                    {skipped !== undefined && (
                        <Action
                            label={t.schedule.restore}
                            icon={Undo2}
                            reason={planReason}
                            pending={pending(t.schedule.restore)}
                            onPress={() =>
                                action.mutate({
                                    label: t.schedule.restore,
                                    call: restoreSkipped,
                                    done: () =>
                                        t.schedule.restored(zoned.clock(skipped.delayed?.to ?? skipped.at)),
                                })
                            }
                            onBlocked={blocked}
                        />
                    )}
                    <Action
                        label={t.schedule.restartOnce}
                        icon={RotateCw}
                        reason={onceReason}
                        onPress={() => setPlanning(true)}
                        onBlocked={blocked}
                    />
                    {schedule.oneOff !== null && (
                        <Action
                            label={t.schedule.cancelOneOff}
                            icon={X}
                            reason={canRestart ? null : t.schedule.notPermittedOnce}
                            pending={pending(t.schedule.cancelOneOff)}
                            onPress={() =>
                                action.mutate({
                                    label: t.schedule.cancelOneOff,
                                    call: cancelOnce,
                                    done: () => t.schedule.cancelled,
                                })
                            }
                            onBlocked={blocked}
                        />
                    )}
                </fieldset>
            </div>
            {planning && (
                <OnceDialog
                    zone={schedule.zone}
                    onClose={() => setPlanning(false)}
                    onPlanned={(status) =>
                        status.oneOff !== null &&
                        setOutcome({
                            tone: 'success',
                            message: t.schedule.planned(
                                `${zoned.day(status.oneOff.at)} ${zoned.clock(status.oneOff.at)}`,
                            ),
                        })
                    }
                />
            )}
        </Card>
    );
}

function OnceDialog({
    zone,
    onClose,
    onPlanned,
}: {
    zone: string;
    onClose: () => void;
    onPlanned: (status: ScheduleStatus) => void;
}) {
    const { t } = useI18n();
    const [mode, setMode] = useState<'in' | 'at'>('in');
    const [minutes, setMinutes] = useState('15');
    const [time, setTime] = useState('');
    const [problem, setProblem] = useState<string | null>(null);
    const hint = useId();
    const plan = useMutation({
        mutationFn: () =>
            restartOnce(mode === 'in' ? { inMinutes: Number(minutes) } : { at: time.slice(0, 5) }),
        onSuccess: (status) => {
            onPlanned(status);
            onClose();
        },
    });

    return (
        <Dialog
            title={t.schedule.onceTitle}
            onClose={onClose}
            closeOnBackdrop={false}
            onSubmit={() => {
                // Said here, by the field: Observer would refuse these as a malformed request.
                const count = Number(minutes);
                if (
                    mode === 'in' &&
                    (minutes.trim() === '' ||
                        !Number.isInteger(count) ||
                        count < 1 ||
                        count > ONCE_MAX_MINUTES)
                ) {
                    setProblem(t.schedule.onceRange);
                } else if (mode === 'at' && minuteOf(time.slice(0, 5)) === null) {
                    setProblem(t.schedule.onceTimeRequired);
                } else {
                    plan.mutate();
                }
            }}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" icon={RotateCw} pending={plan.isPending}>
                        {t.schedule.onceSubmit}
                    </Button>
                </>
            }
        >
            <div className="flex flex-col gap-4" aria-describedby={hint}>
                <Segmented
                    legend={t.schedule.onceWhen}
                    name="once-mode"
                    options={[
                        { value: 'in', label: t.schedule.onceIn },
                        { value: 'at', label: t.schedule.onceAt },
                    ]}
                    value={mode}
                    onChange={(next) => {
                        setMode(next);
                        setProblem(null);
                    }}
                />
                {mode === 'in' ? (
                    <div className="flex flex-col gap-2">
                        <TextField
                            label={t.schedule.onceMinutes}
                            type="number"
                            inputMode="numeric"
                            min={1}
                            max={ONCE_MAX_MINUTES}
                            value={minutes}
                            error={problem ?? undefined}
                            onChange={(event) => {
                                setMinutes(event.target.value);
                                setProblem(null);
                            }}
                        />
                        <div className="flex flex-wrap gap-2">
                            {ONCE_PRESETS.map((preset) => (
                                <Button
                                    key={preset}
                                    size="sm"
                                    tone={String(preset) === minutes ? 'ink' : 'secondary'}
                                    aria-pressed={String(preset) === minutes}
                                    className="max-md:flex-1"
                                    onClick={() => {
                                        setMinutes(String(preset));
                                        setProblem(null);
                                    }}
                                >
                                    {t.schedule.onceMinutesShort(preset)}
                                </Button>
                            ))}
                        </div>
                    </div>
                ) : (
                    <TextField
                        label={t.schedule.onceTime}
                        hint={zone}
                        type="time"
                        className="max-w-48"
                        inputClassName="font-mono"
                        value={time}
                        error={problem ?? undefined}
                        onChange={(event) => {
                            setTime(event.target.value);
                            setProblem(null);
                        }}
                    />
                )}
                <p id={hint} className="text-small text-ink-3">
                    {t.schedule.onceHint}
                </p>
                {plan.isError && <Notice tone="danger">{errorMessage(t, plan.error)}</Notice>}
            </div>
        </Dialog>
    );
}

function Upcoming({
    schedule,
    server,
    now,
}: {
    schedule: ScheduleStatus;
    server: ServerStatus | null;
    now: number;
}) {
    const { t } = useI18n();
    const zoned = useZoned(schedule.zone);
    const offsetNow = zoned.offset(now);
    const waiting =
        schedule.settings.enabled &&
        schedule.settings.mode === 'interval' &&
        !(server !== null && isRunning(server.state));

    return (
        <Card id="schedule-upcoming" icon={History} title={t.schedule.upcomingTitle}>
            {schedule.upcoming.length === 0 ? (
                <p className="text-center text-ui text-ink-3">
                    {waiting ? t.schedule.countsFromStart : t.schedule.upcomingEmpty}
                </p>
            ) : (
                <ul className="flex flex-col divide-y divide-line">
                    {schedule.upcoming.map((occurrence) => {
                        const clock = zoned.clock(occurrence.at);
                        const offset = zoned.offset(occurrence.at);
                        const moved = occurrence.time !== null && occurrence.time !== clock;
                        return (
                            <li
                                key={occurrence.at}
                                className="flex flex-wrap items-center gap-x-4 gap-y-1.5 py-2.5 first:pt-0 last:pb-0"
                            >
                                <span className="w-28 text-ui text-ink-2">{zoned.day(occurrence.at)}</span>
                                <span
                                    className={cn(
                                        'font-mono text-ui tabular-nums',
                                        occurrence.skipped === null ? 'text-ink' : 'text-ink-3 line-through',
                                    )}
                                >
                                    {clock}
                                    {offset !== offsetNow && (
                                        <span className="ml-1.5 text-ink-3">{offset}</span>
                                    )}
                                </span>
                                <span className="flex flex-wrap gap-1.5">
                                    {occurrence.skipped?.kind === 'manual' && (
                                        <Chip>{t.schedule.skippedBy(occurrence.skipped.by)}</Chip>
                                    )}
                                    {occurrence.skipped?.kind === 'recentStart' && (
                                        <Chip>
                                            {t.schedule.skippedStart(
                                                zoned.clock(occurrence.skipped.startedAt),
                                            )}
                                        </Chip>
                                    )}
                                    {occurrence.delayed !== null && (
                                        <Chip>
                                            {t.schedule.delayedTo(zoned.clock(occurrence.delayed.to))}
                                        </Chip>
                                    )}
                                    {moved && <Chip>{t.schedule.moved}</Chip>}
                                </span>
                            </li>
                        );
                    })}
                </ul>
            )}
        </Card>
    );
}

function Segmented<T extends string>({
    legend,
    name,
    options,
    value,
    onChange,
}: {
    legend: string;
    name: string;
    options: { value: T; label: string }[];
    value: T;
    onChange: (value: T) => void;
}) {
    return (
        <fieldset className="flex min-w-0 flex-col">
            <legend className="mb-1.5 text-ui font-medium text-ink">{legend}</legend>
            <div className="inline-flex max-w-full gap-0.5 self-start rounded-control border border-line bg-surface-2 p-0.75">
                {options.map((option) => (
                    <label
                        key={option.value}
                        className="inline-flex h-6.5 cursor-pointer items-center justify-center rounded-[7px] px-2.5 text-small font-medium text-ink-3 transition-colors hover:text-ink has-checked:bg-raise has-checked:text-ink has-checked:shadow-[var(--elev-sm),0_0_0_1px_var(--line)] has-focus-visible:outline-2 has-focus-visible:outline-offset-1 has-focus-visible:outline-accent max-md:h-11 max-md:min-w-11 max-md:px-3.5"
                    >
                        <input
                            type="radio"
                            name={name}
                            value={option.value}
                            className="sr-only"
                            checked={value === option.value}
                            onChange={() => onChange(option.value)}
                        />
                        {option.label}
                    </label>
                ))}
            </div>
        </fieldset>
    );
}

function Switch({
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
        <div className="flex items-start gap-3">
            <SwitchTrack
                id={id}
                checked={checked}
                aria-labelledby={`${id}-name`}
                aria-describedby={`${id}-about`}
                onChange={(event) => onChange(event.target.checked)}
            />
            <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                <label
                    id={`${id}-name`}
                    htmlFor={id}
                    className="cursor-pointer text-body font-medium text-ink"
                >
                    {label}
                </label>
                <span id={`${id}-about`} className="text-small text-ink-2">
                    {description}
                </span>
            </span>
        </div>
    );
}

/** The native checkbox as a switch: a 36 × 20 track, its knob moving over when on (DESIGN.md §8.16). */
function SwitchTrack({
    checked,
    ...props
}: Omit<ComponentProps<'input'>, 'type' | 'role'> & { checked: boolean }) {
    return (
        <span className="relative mt-0.5 inline-flex h-5 w-9 flex-none">
            <input
                type="checkbox"
                role="switch"
                aria-checked={checked}
                checked={checked}
                className="peer absolute inset-0 m-0 cursor-pointer opacity-0"
                {...props}
            />
            <span
                aria-hidden="true"
                className="pointer-events-none absolute inset-0 rounded-pill border border-line-strong bg-surface-3 transition-colors peer-checked:border-btn peer-checked:bg-btn peer-focus-visible:outline-2 peer-focus-visible:outline-offset-2 peer-focus-visible:outline-accent peer-disabled:opacity-45"
            />
            <span
                aria-hidden="true"
                className="pointer-events-none absolute top-[3px] left-[3px] size-3.5 rounded-pill bg-ink-3 transition-[transform,background-color] duration-200 peer-checked:translate-x-4 peer-checked:bg-btn-ink"
            />
        </span>
    );
}

/** A removable chip: a time or a warning of the settings. */
function ListChip({
    label,
    remove,
    invalid,
    onRemove,
}: {
    label: string;
    remove: string;
    invalid: boolean;
    onRemove: () => void;
}) {
    return (
        <span
            className={cn(
                chipVariants({ tone: invalid ? 'danger' : 'neutral' }),
                'h-7 gap-1 pr-1 font-mono text-ui max-md:h-11 max-md:pl-3',
            )}
        >
            {label}
            <button
                type="button"
                className="grid size-5 place-items-center rounded-pill text-ink-3 hover:bg-hover hover:text-ink focus-visible:outline-2 focus-visible:outline-accent max-md:size-9"
                aria-label={remove}
                onClick={onRemove}
            >
                <X size={12} strokeWidth={2} aria-hidden="true" />
            </button>
        </span>
    );
}

// Browsers list no `UTC`, which Observer knows: a host in another zone could not be set to it.
const zones = (() => {
    try {
        return ['UTC', ...Intl.supportedValuesOf('timeZone').filter((zone) => zone !== 'UTC')];
    } catch {
        return ['UTC'];
    }
})();

/**
 * The numbers Observer takes as whole only, checked before sending: a fraction, or an emptied field,
 * would be refused as a malformed request rather than next to its field.
 */
const wholeNumberIssues = (settings: ScheduleSettings): ScheduleIssue[] =>
    (['intervalMinutes', 'skipAfterStart'] as const)
        .filter((field) => !Number.isInteger(settings[field]))
        .map((field) => ({ field, index: null, problem: 'outOfRange' }));

function SettingsForm({ schedule }: { schedule: ScheduleStatus }) {
    const { t } = useI18n();
    const me = useMe();
    const canEdit = allows(me, Permission.ScheduleManage);
    // What the draft was started from: the settings as stored then, and the version a save names.
    const [shown, setShown] = useState({ settings: schedule.settings, version: schedule.version });
    const [draft, setDraft] = useState<ScheduleSettings>(schedule.settings);
    const [issues, setIssues] = useState<ScheduleIssue[]>([]);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const [newTime, setNewTime] = useState('');
    const [every, setEvery] = useState(6);
    const [from, setFrom] = useState('04:00');
    const [newWarning, setNewWarning] = useState('');
    const notice = useRef<HTMLDivElement>(null);
    const form = useRef<HTMLFormElement>(null);
    const showFirstIssue = useRef(false);

    const changes = countChanges(shown.settings, draft);
    const stale = schedule.version !== shown.version;
    const startFrom = (status: ScheduleStatus) => {
        setShown({ settings: status.settings, version: status.version });
        setDraft(status.settings);
    };
    // Someone saved since: with nothing typed yet, show what is stored now rather than what it
    // replaced, which a save would quietly bring back.
    if (stale && changes === 0) {
        startFrom(schedule);
    }

    const save = useMutation({
        mutationFn: () => saveSchedule({ settings: draft, version: shown.version }),
        onSuccess: (status) => {
            startFrom(status);
            setIssues([]);
            setOutcome({ tone: 'success', message: t.schedule.saved });
        },
        onError: (error) => {
            const conflict = error instanceof ApiError && error.code === 'conflict';
            const refused = error instanceof ApiError && error.code === 'invalid_schedule';
            showFirstIssue.current = refused;
            setIssues(refused ? scheduleIssues(error.params) : []);
            setOutcome({
                tone: conflict ? 'warning' : 'danger',
                message: conflict ? t.schedule.changedElsewhere : errorMessage(t, error),
            });
        },
    });

    const issue = (field: ScheduleField, index: number | null = null) => {
        const found = issues.find((each) => each.field === field && each.index === index);
        if (found === undefined) {
            return undefined;
        }
        if (found.problem === 'outOfRange' || found.problem === 'tooMany') {
            const ranges: Partial<Record<ScheduleField, string>> = t.schedule.ranges;
            return ranges[field] ?? t.schedule.problems[found.problem];
        }
        return t.schedule.problems[found.problem];
    };
    const edit = (change: Partial<ScheduleSettings>) => {
        setDraft((current) => ({ ...current, ...change }));
        setIssues([]);
        setOutcome(null);
    };
    const addTime = (time: string) => {
        const value = time.slice(0, 5);
        if (minuteOf(value) !== null && !draft.times.includes(value)) {
            edit({ times: sortTimes([...draft.times, value]) });
        }
        setNewTime('');
    };
    const addWarning = () => {
        const minutes = Number(newWarning);
        if (Number.isInteger(minutes) && minutes > 0 && !draft.warnings.includes(minutes)) {
            edit({ warnings: [...draft.warnings, minutes].toSorted((a, b) => b - a) });
        }
        setNewWarning('');
    };
    const discard = () => {
        startFrom(schedule);
        setIssues([]);
        setOutcome({ tone: 'neutral', message: t.schedule.discarded });
    };

    useEffect(() => {
        if (outcome !== null && document.activeElement === document.body) {
            notice.current?.focus();
        }
    }, [outcome]);

    useEffect(() => {
        if (!showFirstIssue.current) {
            return;
        }
        showFirstIssue.current = false;
        form.current?.querySelector<HTMLElement>('[aria-invalid="true"], [data-invalid]')?.scrollIntoView({
            block: 'center',
        });
    });

    const submit = (event: FormEvent) => {
        event.preventDefault();
        setOutcome(null);
        const local = wholeNumberIssues(draft);
        if (local.length > 0) {
            showFirstIssue.current = true;
            setIssues(local);
        } else if (!save.isPending) {
            save.mutate();
        }
    };

    const timeIssues = draft.times
        .map((time, index) => ({ time, message: issue('time', index) }))
        .filter((each) => each.message !== undefined);
    const warningIssues = draft.warnings
        .map((minutes, index) => ({ minutes, message: issue('warning', index) }))
        .filter((each) => each.message !== undefined);
    const hours = Math.floor(draft.intervalMinutes / 60);
    const minutesPart = draft.intervalMinutes % 60;
    const hostZone = schedule.settings.timeZone === null ? schedule.zone : null;
    const zoneOptions =
        draft.timeZone !== null && !zones.includes(draft.timeZone) ? [draft.timeZone, ...zones] : zones;

    return (
        <form ref={form} className="flex flex-col gap-3" onSubmit={submit} noValidate>
            <Card id="schedule-settings" icon={CalendarClock} title={t.schedule.settingsTitle}>
                <fieldset disabled={!canEdit} className="flex min-w-0 max-w-2xl flex-col gap-5">
                    {!canEdit && <Notice>{t.schedule.readOnly}</Notice>}
                    {stale && outcome === null && (
                        <Notice tone="warning">{t.schedule.changedElsewhere}</Notice>
                    )}
                    {outcome && (
                        <Notice ref={notice} tabIndex={-1} tone={outcome.tone}>
                            {outcome.message}
                        </Notice>
                    )}
                    <Switch
                        label={t.schedule.enabled}
                        description={t.schedule.enabledHint}
                        checked={draft.enabled}
                        onChange={(enabled) => edit({ enabled })}
                    />
                    <Segmented
                        legend={t.schedule.mode}
                        name="schedule-mode"
                        options={MODES.map((mode) => ({ value: mode, label: t.schedule.modes[mode] }))}
                        value={draft.mode}
                        onChange={(mode) => edit({ mode })}
                    />

                    {draft.mode === 'fixedTimes' ? (
                        <div className="flex flex-col gap-3">
                            <div className="flex flex-col gap-1.5">
                                <span className="text-ui font-medium text-ink">{t.schedule.times}</span>
                                {draft.times.length === 0 ? (
                                    <p className="text-small text-ink-3">{t.schedule.noTimes}</p>
                                ) : (
                                    <ul className="flex flex-wrap gap-1.5" aria-label={t.schedule.times}>
                                        {draft.times.map((time, index) => (
                                            <li
                                                key={time}
                                                data-invalid={issue('time', index) ? true : undefined}
                                            >
                                                <ListChip
                                                    label={time}
                                                    remove={t.schedule.removeTime(time)}
                                                    invalid={issue('time', index) !== undefined}
                                                    onRemove={() =>
                                                        edit({
                                                            times: draft.times.filter(
                                                                (other) => other !== time,
                                                            ),
                                                        })
                                                    }
                                                />
                                            </li>
                                        ))}
                                    </ul>
                                )}
                                {issue('times') && <FieldError>{issue('times')}</FieldError>}
                                {timeIssues.map(({ time, message }) => (
                                    <FieldError key={time}>{`${time}: ${message}`}</FieldError>
                                ))}
                            </div>
                            <div className="flex flex-wrap items-end gap-2">
                                <TextField
                                    label={t.schedule.addTime}
                                    type="time"
                                    className="w-40 max-md:flex-1"
                                    inputClassName="font-mono"
                                    value={newTime}
                                    onChange={(event) => setNewTime(event.target.value)}
                                    onKeyDown={(event) => {
                                        if (event.key === 'Enter') {
                                            event.preventDefault();
                                            addTime(newTime);
                                        }
                                    }}
                                />
                                <Button
                                    icon={Plus}
                                    className="max-md:flex-1"
                                    onClick={() => addTime(newTime)}
                                >
                                    {t.schedule.add}
                                </Button>
                            </div>
                            <fieldset className="flex min-w-0 flex-col gap-1.5">
                                <legend className="sr-only">{t.schedule.fillLegend}</legend>
                                <div className="flex flex-wrap items-center gap-2 text-ui text-ink-2">
                                    <span>{t.schedule.every}</span>
                                    <select
                                        aria-label={t.schedule.everyHours}
                                        className={cn(inputClass, 'w-20 font-mono')}
                                        value={every}
                                        onChange={(event) => setEvery(Number(event.target.value))}
                                    >
                                        {EVERY_HOURS.map((step) => (
                                            <option key={step} value={step}>
                                                {step}
                                            </option>
                                        ))}
                                    </select>
                                    <span>{t.schedule.hoursFrom}</span>
                                    <input
                                        type="time"
                                        aria-label={t.schedule.from}
                                        className={cn(inputClass, 'w-32 font-mono')}
                                        value={from}
                                        onChange={(event) => setFrom(event.target.value)}
                                    />
                                    <Button
                                        size="sm"
                                        className="max-md:flex-1"
                                        onClick={() => edit({ times: fillTimes(every, from.slice(0, 5)) })}
                                    >
                                        {t.schedule.fill}
                                    </Button>
                                </div>
                                <p className="text-small text-ink-3">{t.schedule.fillHint}</p>
                            </fieldset>
                        </div>
                    ) : (
                        <fieldset className="flex min-w-0 flex-col gap-1.5">
                            <legend className="mb-1.5 text-ui font-medium text-ink">
                                {t.schedule.modes.interval}
                            </legend>
                            <div className="flex flex-wrap gap-3">
                                <TextField
                                    label={t.schedule.intervalHours}
                                    type="number"
                                    inputMode="numeric"
                                    min={0}
                                    max={168}
                                    className="w-28 max-md:flex-1"
                                    value={hours}
                                    aria-invalid={issue('intervalMinutes') ? true : undefined}
                                    onChange={(event) =>
                                        edit({
                                            intervalMinutes: Number(event.target.value) * 60 + minutesPart,
                                        })
                                    }
                                />
                                <TextField
                                    label={t.schedule.intervalMinutes}
                                    type="number"
                                    inputMode="numeric"
                                    min={0}
                                    max={59}
                                    className="w-28 max-md:flex-1"
                                    value={minutesPart}
                                    onChange={(event) =>
                                        edit({ intervalMinutes: hours * 60 + Number(event.target.value) })
                                    }
                                />
                            </div>
                            {issue('intervalMinutes') && <FieldError>{issue('intervalMinutes')}</FieldError>}
                            <p className="text-small text-ink-3">{t.schedule.intervalHint}</p>
                        </fieldset>
                    )}

                    <div className="flex min-w-0 flex-col gap-1.5">
                        <label htmlFor="schedule-zone" className="text-ui font-medium text-ink">
                            {t.schedule.timeZone}
                        </label>
                        <select
                            id="schedule-zone"
                            className={cn(inputClass, 'max-w-sm')}
                            value={draft.timeZone ?? ''}
                            aria-invalid={issue('timeZone') ? true : undefined}
                            onChange={(event) => edit({ timeZone: event.target.value || null })}
                        >
                            <option value="">{t.schedule.hostZone(hostZone)}</option>
                            {zoneOptions.map((zone) => (
                                <option key={zone} value={zone}>
                                    {zone}
                                </option>
                            ))}
                        </select>
                        {issue('timeZone') ? (
                            <FieldError>{issue('timeZone')}</FieldError>
                        ) : (
                            <p className="text-small text-ink-3">{t.schedule.timeZoneHint}</p>
                        )}
                    </div>

                    <div className="flex flex-col gap-1.5">
                        <span className="text-ui font-medium text-ink">{t.schedule.warnings}</span>
                        <ul className="flex flex-wrap gap-1.5" aria-label={t.schedule.warnings}>
                            {draft.warnings.map((minutes, index) => (
                                <li key={minutes} data-invalid={issue('warning', index) ? true : undefined}>
                                    <ListChip
                                        label={t.schedule.warningMinutes(minutes)}
                                        remove={t.schedule.removeWarning(minutes)}
                                        invalid={issue('warning', index) !== undefined}
                                        onRemove={() =>
                                            edit({
                                                warnings: draft.warnings.filter((other) => other !== minutes),
                                            })
                                        }
                                    />
                                </li>
                            ))}
                        </ul>
                        {issue('warnings') && <FieldError>{issue('warnings')}</FieldError>}
                        {warningIssues.map(({ minutes, message }) => (
                            <FieldError
                                key={minutes}
                            >{`${t.schedule.warningMinutes(minutes)}: ${message}`}</FieldError>
                        ))}
                        <div className="flex flex-wrap items-end gap-2">
                            <TextField
                                label={t.schedule.addWarning}
                                type="number"
                                inputMode="numeric"
                                min={1}
                                max={240}
                                className="w-56 max-md:flex-1"
                                value={newWarning}
                                onChange={(event) => setNewWarning(event.target.value)}
                                onKeyDown={(event) => {
                                    if (event.key === 'Enter') {
                                        event.preventDefault();
                                        addWarning();
                                    }
                                }}
                            />
                            <Button icon={Plus} className="max-md:flex-1" onClick={addWarning}>
                                {t.schedule.add}
                            </Button>
                            <Button
                                tone="ghost"
                                className="max-md:flex-1"
                                onClick={() => edit({ warnings: DEFAULT_WARNINGS })}
                            >
                                {t.schedule.resetWarnings}
                            </Button>
                        </div>
                        <p className="text-small text-ink-3">{t.schedule.warningsHint}</p>
                    </div>

                    <TextField
                        label={t.schedule.skipAfterStart}
                        hint={t.schedule.skipAfterStartHint}
                        type="number"
                        inputMode="numeric"
                        min={0}
                        max={720}
                        className="max-w-md"
                        value={Number.isNaN(draft.skipAfterStart) ? '' : draft.skipAfterStart}
                        error={issue('skipAfterStart')}
                        onChange={(event) =>
                            edit({
                                skipAfterStart:
                                    event.target.value === '' ? Number.NaN : Number(event.target.value),
                            })
                        }
                    />
                </fieldset>
            </Card>

            {canEdit && changes > 0 && (
                <div className="sticky bottom-3 z-10 flex flex-wrap items-center gap-x-4 gap-y-2 rounded-box border border-line-strong bg-surface px-3.5 py-2.5 shadow-pop">
                    <output className="mr-auto text-ui text-ink-2">{t.schedule.changes(changes)}</output>
                    <Button tone="ghost" onClick={discard} className="max-md:flex-1">
                        {t.schedule.discard}
                    </Button>
                    <Button type="submit" tone="ink" pending={save.isPending} className="max-md:flex-1">
                        {t.schedule.save}
                    </Button>
                </div>
            )}
        </form>
    );
}
