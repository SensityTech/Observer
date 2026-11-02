import { useMutation } from '@tanstack/react-query';
import {
    CalendarClock,
    CalendarX2,
    Clock,
    History,
    RotateCw,
    SkipForward,
    Undo2,
    X,
    type LucideIcon,
} from 'lucide-react';
import { useEffect, useId, useRef, useState } from 'react';

import { Permission } from '../api/generated/Permission';
import type { ScheduleStatus } from '../api/generated/ScheduleStatus';
import type { ServerStatus } from '../api/generated/ServerStatus';
import { cancelOnce, delayNext, restartOnce, restoreSkipped, skipNext } from '../api/schedule';
import { allows, useMe } from '../auth/me';
import { Button, buttonVariants } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { Dialog } from '../components/Dialog';
import { TextField } from '../components/Field';
import { Menu } from '../components/Menu';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { formatClock, formatDay, knownZone, zoneOffset } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import { cn } from '../lib/cn';
import { useNow } from '../lib/useNow';
import { useSchedule } from '../schedule/context';
import { minuteOf } from '../schedule/times';
import { useServerStatus } from '../server/context';
import { isRunning } from '../server/state';
import { MovedSettings } from '../settings/Moved';

const DELAYS = [15, 30, 60, 120];
const ONCE_PRESETS = [5, 15, 30];
const ONCE_MAX_MINUTES = 1440;

type Outcome = {
    tone: 'neutral' | 'success' | 'warning' | 'danger';
    message: string;
    /** The next restart and the single one it was said of, by `restartsOf`. */
    about?: string;
};

/** What an outcome of the Next restart card holds for: once these move on, it no longer does. */
const restartsOf = (status: ScheduleStatus) => `${status.next?.at ?? ''} ${status.oneOff?.at ?? ''}`;

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
                    <MovedSettings category="restarts" page="schedule" />
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
    const notice = useRef<HTMLDivElement>(null);
    // An action pressed from the keyboard loses it: its button is disabled while it runs, and Restore
    // or Cancel goes once done. The outcome gets it then.
    const refocus = useRef(false);
    const canPlan = allows(me, Permission.ScheduleManage);
    const canRestart = allows(me, Permission.ServerControl);
    const running = server !== null && server.state !== 'stopped' && server.state !== 'crashLoop';
    const next = schedule.next;
    const skipped = schedule.upcoming.find((occurrence) => occurrence.skipped?.kind === 'manual');
    const restarts = restartsOf(schedule);
    const [seen, setSeen] = useState(restarts);
    if (seen !== restarts) {
        setSeen(restarts);
        if (outcome !== null && outcome.about !== restarts) {
            setOutcome(null);
        }
    }

    const action = useMutation({
        mutationFn: ({
            call,
        }: {
            label: string;
            call: () => Promise<ScheduleStatus>;
            done: (status: ScheduleStatus) => string;
        }) => call(),
        onMutate: () => {
            refocus.current = document.activeElement !== document.body;
            setOutcome(null);
        },
        onSuccess: (status, { done }) =>
            setOutcome({ tone: 'success', message: done(status), about: restartsOf(status) }),
        onError: (error, { label }) =>
            setOutcome({
                tone: 'danger',
                message: t.schedule.failed(label, errorMessage(t, error)),
                about: restarts,
            }),
    });
    const blocked = (label: string, reason: string) =>
        setOutcome({ tone: 'warning', message: t.schedule.unavailable(label, reason), about: restarts });
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
            : server.state === 'hung'
              ? t.schedule.hungOnce
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

    // After every render: the button may go with the status that follows the answer.
    useEffect(() => {
        if (refocus.current && outcome !== null && document.activeElement === document.body) {
            refocus.current = false;
            notice.current?.focus();
        }
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
                    <Notice ref={notice} tabIndex={-1} tone={outcome.tone}>
                        <div className="flex items-start gap-3">
                            <p className="flex-1">{outcome.message}</p>
                            <button
                                type="button"
                                className="-m-1 grid place-items-center rounded-sm p-1 max-md:-my-3 max-md:-mr-3 max-md:size-11 max-md:p-0"
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
                            about: restartsOf(status),
                            message: t.schedule.planned(
                                t.schedule.dayTime(
                                    zoned.day(status.oneOff.at),
                                    zoned.clock(status.oneOff.at),
                                ),
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
            <div className="flex flex-col gap-4">
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
                            describedBy={hint}
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
                        describedBy={hint}
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
    // A browser without the zone shows UTC times, which tell nothing of a clock change there.
    const known = knownZone(schedule.zone) === schedule.zone;
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
                        const moved = known && occurrence.time !== null && occurrence.time !== clock;
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
