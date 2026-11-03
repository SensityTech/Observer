import { useMutation } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Plus, X } from 'lucide-react';
import { useEffect, useRef, useState, type Ref } from 'react';

import { ApiError } from '../api/client';
import { Permission } from '../api/generated/Permission';
import type { ScheduleField } from '../api/generated/ScheduleField';
import type { ScheduleIssue } from '../api/generated/ScheduleIssue';
import type { ScheduleMode } from '../api/generated/ScheduleMode';
import type { ScheduleSettings } from '../api/generated/ScheduleSettings';
import type { ScheduleStatus } from '../api/generated/ScheduleStatus';
import { changedSettings, DEFAULT_WARNINGS, saveSchedule, scheduleIssues } from '../api/schedule';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { chipVariants } from '../components/Chip';
import { FieldError, inputClass, TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { Switch } from '../components/Switch';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { useLinkState } from '../live/context';
import { cn } from '../lib/cn';
import { useSchedule } from '../schedule/context';
import { EVERY_HOURS, fillTimes, minuteOf, sortTimes } from '../schedule/times';
import type { SettingId } from './catalog';
import { keyboardLeft, SettingsForm, useSection } from './Form';
import { CategoryHead, Segmented, SettingRow, SettingsGroup } from './Row';

const MODES: ScheduleMode[] = ['fixedTimes', 'interval'];
const MAX_WARNING_MINUTES = 240;

type Outcome = { tone: 'neutral' | 'success' | 'warning' | 'danger'; message: string };

/** The setting each stored field is shown as. */
const SETTING_OF = {
    enabled: 'scheduleEnabled',
    mode: 'scheduleMode',
    times: 'scheduleTimes',
    intervalMinutes: 'scheduleInterval',
    timeZone: 'timeZone',
    warnings: 'warnings',
    skipAfterStart: 'skipAfterStart',
} satisfies Record<keyof ScheduleSettings, SettingId>;

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

/** The restart schedule's settings, from the live schedule: what sat at the bottom of the Schedule page. */
export function RestartSettings() {
    const { t } = useI18n();
    const link = useLinkState();
    const { schedule: live, refused } = useSchedule();
    // While the link is down the category keeps the last status it had, so that the form, and what
    // is typed in it, stays.
    const [last, setLast] = useState(live);
    if (live !== null && live !== last) {
        setLast(live);
    }
    const schedule = live ?? (refused ? null : last);

    return (
        <>
            <CategoryHead
                category="restarts"
                aside={
                    <Link
                        to="/schedule"
                        className="inline-flex items-center gap-1.5 text-ui font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                    >
                        {t.settings.restarts.openSchedule}
                        <ArrowRight size={14} strokeWidth={1.5} aria-hidden="true" />
                    </Link>
                }
            />
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
                <SettingsForm>
                    <ScheduleForm schedule={schedule} />
                </SettingsForm>
            )}
        </>
    );
}

function ScheduleForm({ schedule }: { schedule: ScheduleStatus }) {
    const { t } = useI18n();
    const words = t.settings.restarts;
    const me = useMe();
    const canEdit = allows(me, Permission.ScheduleManage);
    // What the draft was started from: the settings as stored then, and the version a save names.
    const [shown, setShown] = useState({ settings: schedule.settings, version: schedule.version });
    const [draft, setDraft] = useState<ScheduleSettings>(schedule.settings);
    const [issues, setIssues] = useState<ScheduleIssue[]>([]);
    const [outcome, setOutcome] = useState<Outcome | null>(null);
    const [newTime, setNewTime] = useState('');
    const [timeProblem, setTimeProblem] = useState<string | null>(null);
    // A new field for each time added: Chrome keeps the minutes of a cleared time field focused,
    // so the next time typed would start there.
    const [timeField, setTimeField] = useState(0);
    const [every, setEvery] = useState(6);
    const [from, setFrom] = useState('04:00');
    const [newWarning, setNewWarning] = useState('');
    const [warningProblem, setWarningProblem] = useState<string | null>(null);
    const notice = useRef<HTMLDivElement>(null);
    const form = useRef<HTMLFieldSetElement>(null);
    const showFirstIssue = useRef(false);
    const removers = useRef(new Map<string, HTMLButtonElement>());
    const addTimeField = useRef<HTMLInputElement>(null);
    const addWarningField = useRef<HTMLInputElement>(null);
    /** Where the keyboard goes once a removed chip is gone: the next chip, or its list's field. */
    const focusAfterRemoval = useRef<{ chip: string | undefined; field: 'time' | 'warning' } | null>(null);
    const remover = (key: string) => (button: HTMLButtonElement | null) => {
        if (button === null) {
            removers.current.delete(key);
        } else {
            removers.current.set(key, button);
        }
    };

    const changed = changedSettings(shown.settings, draft);
    const stale = schedule.version !== shown.version;
    const startFrom = (status: ScheduleStatus) => {
        setShown({ settings: status.settings, version: status.version });
        setDraft(status.settings);
    };
    // Someone saved since: with nothing typed yet, show what is stored now rather than what it
    // replaced, which a save would quietly bring back.
    if (stale && changed.length === 0) {
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

    useSection({
        changed: canEdit ? changed.map((key) => SETTING_OF[key]) : [],
        pending: save.isPending,
        save: () => {
            setOutcome(null);
            const local = wholeNumberIssues(draft);
            if (local.length > 0) {
                showFirstIssue.current = true;
                setIssues(local);
            } else {
                save.mutate();
            }
        },
        discard: () => {
            startFrom(schedule);
            setIssues([]);
            setOutcome({ tone: 'neutral', message: t.schedule.discarded });
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
        if (minuteOf(value) === null) {
            setTimeProblem(t.schedule.timeIncomplete);
        } else if (draft.times.includes(value)) {
            setTimeProblem(t.schedule.listedAlready);
        } else {
            edit({ times: sortTimes([...draft.times, value]) });
            setNewTime('');
            setTimeField((key) => key + 1);
        }
    };
    const addWarning = () => {
        const minutes = Number(newWarning);
        if (
            newWarning.trim() === '' ||
            !Number.isInteger(minutes) ||
            minutes < 1 ||
            minutes > MAX_WARNING_MINUTES
        ) {
            setWarningProblem(t.schedule.ranges.warning);
        } else if (draft.warnings.includes(minutes)) {
            setWarningProblem(t.schedule.listedAlready);
        } else {
            edit({ warnings: [...draft.warnings, minutes].toSorted((a, b) => b - a) });
            setNewWarning('');
        }
    };

    useEffect(() => {
        if (outcome !== null && keyboardLeft()) {
            notice.current?.focus();
        }
    }, [outcome]);

    useEffect(() => {
        if (timeField > 0) {
            addTimeField.current?.focus();
        }
    }, [timeField]);

    useEffect(() => {
        const target = focusAfterRemoval.current;
        if (target === null) {
            return;
        }
        focusAfterRemoval.current = null;
        const chip = target.chip === undefined ? undefined : removers.current.get(target.chip);
        (chip ?? (target.field === 'time' ? addTimeField : addWarningField).current)?.focus();
    });

    useEffect(() => {
        if (!showFirstIssue.current) {
            return;
        }
        showFirstIssue.current = false;
        form.current?.querySelector<HTMLElement>('[aria-invalid="true"], [data-invalid]')?.scrollIntoView({
            block: 'center',
        });
    });

    const timeIssues = draft.times
        .map((time, index) => ({ time, message: issue('time', index) }))
        .filter((each) => each.message !== undefined);
    const warningIssues = draft.warnings
        .map((minutes, index) => ({ minutes, message: issue('warning', index) }))
        .filter((each) => each.message !== undefined);
    const intervalDescribedBy = issue('intervalMinutes')
        ? 'schedule-interval-error schedule-interval-hint'
        : 'schedule-interval-hint';
    const hours = Math.floor(draft.intervalMinutes / 60);
    const minutesPart = draft.intervalMinutes % 60;
    const hostZone = schedule.settings.timeZone === null ? schedule.zone : null;
    const zoneOptions =
        draft.timeZone !== null && !zones.includes(draft.timeZone) ? [draft.timeZone, ...zones] : zones;
    const is = (key: keyof ScheduleSettings) => canEdit && changed.includes(key);

    return (
        <fieldset ref={form} disabled={!canEdit} className="flex min-w-0 flex-col gap-4">
            {!canEdit && <Notice>{t.schedule.readOnly}</Notice>}
            {stale && outcome === null && <Notice tone="warning">{t.schedule.changedElsewhere}</Notice>}
            {outcome && (
                <Notice ref={notice} tabIndex={-1} tone={outcome.tone}>
                    {outcome.message}
                </Notice>
            )}

            <SettingsGroup title={words.groups.when}>
                <SettingRow id="scheduleEnabled" field changed={is('enabled')}>
                    {(ids) => (
                        <Switch
                            id={ids.control}
                            aria-describedby={ids.about}
                            checked={draft.enabled}
                            onChange={(event) => edit({ enabled: event.target.checked })}
                        />
                    )}
                </SettingRow>
                <SettingRow id="scheduleMode" changed={is('mode')}>
                    {(ids) => (
                        <Segmented
                            ids={ids}
                            name="schedule-mode"
                            choices={MODES}
                            words={t.schedule.modes}
                            value={draft.mode}
                            onChange={(mode) => edit({ mode })}
                        />
                    )}
                </SettingRow>

                {draft.mode === 'fixedTimes' ? (
                    <SettingRow id="scheduleTimes" changed={is('times')}>
                        {(ids) => (
                            <fieldset aria-labelledby={ids.label} className="min-w-0 flex flex-col gap-3">
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
                                                    removeRef={remover(`time ${time}`)}
                                                    onRemove={() => {
                                                        const next =
                                                            draft.times[index + 1] ?? draft.times[index - 1];
                                                        focusAfterRemoval.current = {
                                                            chip:
                                                                next === undefined
                                                                    ? undefined
                                                                    : `time ${next}`,
                                                            field: 'time',
                                                        };
                                                        edit({
                                                            times: draft.times.filter(
                                                                (other) => other !== time,
                                                            ),
                                                        });
                                                    }}
                                                />
                                            </li>
                                        ))}
                                    </ul>
                                )}
                                {issue('times') && <FieldError>{issue('times')}</FieldError>}
                                {timeIssues.map(({ time, message }) => (
                                    <FieldError key={time}>
                                        {t.schedule.issueOf(time, message ?? '')}
                                    </FieldError>
                                ))}
                                <div className="flex flex-wrap items-end gap-2">
                                    <TextField
                                        key={timeField}
                                        ref={addTimeField}
                                        label={t.schedule.addTime}
                                        type="time"
                                        className="w-40 max-md:flex-1"
                                        inputClassName="font-mono"
                                        value={newTime}
                                        error={timeProblem ?? undefined}
                                        onChange={(event) => {
                                            setNewTime(event.target.value);
                                            setTimeProblem(null);
                                        }}
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
                                            onClick={() =>
                                                edit({ times: fillTimes(every, from.slice(0, 5)) })
                                            }
                                        >
                                            {t.schedule.fill}
                                        </Button>
                                    </div>
                                    <p className="text-small text-ink-3">{t.schedule.fillHint}</p>
                                </fieldset>
                            </fieldset>
                        )}
                    </SettingRow>
                ) : (
                    <SettingRow id="scheduleInterval" changed={is('intervalMinutes')}>
                        {(ids) => (
                            <fieldset aria-labelledby={ids.label} className="min-w-0 flex flex-col gap-1.5">
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
                                        describedBy={intervalDescribedBy}
                                        onChange={(event) =>
                                            edit({
                                                intervalMinutes:
                                                    Number(event.target.value) * 60 + minutesPart,
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
                                        aria-invalid={issue('intervalMinutes') ? true : undefined}
                                        describedBy={intervalDescribedBy}
                                        onChange={(event) =>
                                            edit({ intervalMinutes: hours * 60 + Number(event.target.value) })
                                        }
                                    />
                                </div>
                                {issue('intervalMinutes') && (
                                    <FieldError id="schedule-interval-error">
                                        {issue('intervalMinutes')}
                                    </FieldError>
                                )}
                                <p id="schedule-interval-hint" className="text-small text-ink-3">
                                    {t.schedule.intervalHint}
                                </p>
                            </fieldset>
                        )}
                    </SettingRow>
                )}

                <SettingRow id="timeZone" field changed={is('timeZone')}>
                    {(ids) => (
                        <>
                            <select
                                id={ids.control}
                                className={cn(inputClass, 'max-w-sm')}
                                value={draft.timeZone ?? ''}
                                aria-invalid={issue('timeZone') ? true : undefined}
                                aria-describedby={
                                    issue('timeZone')
                                        ? `schedule-zone-error ${ids.about}`
                                        : `${ids.about} schedule-zone-hint`
                                }
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
                                <FieldError id="schedule-zone-error">{issue('timeZone')}</FieldError>
                            ) : (
                                <p id="schedule-zone-hint" className="text-small text-ink-3">
                                    {t.schedule.timeZoneHint}
                                </p>
                            )}
                        </>
                    )}
                </SettingRow>
            </SettingsGroup>

            <SettingsGroup title={words.groups.around}>
                <SettingRow id="warnings" changed={is('warnings')}>
                    {(ids) => (
                        <fieldset aria-labelledby={ids.label} className="min-w-0 flex flex-col gap-3">
                            <ul className="flex flex-wrap gap-1.5" aria-label={t.schedule.warnings}>
                                {draft.warnings.map((minutes, index) => (
                                    <li
                                        key={minutes}
                                        data-invalid={issue('warning', index) ? true : undefined}
                                    >
                                        <ListChip
                                            label={t.schedule.warningMinutes(minutes)}
                                            remove={t.schedule.removeWarning(minutes)}
                                            invalid={issue('warning', index) !== undefined}
                                            removeRef={remover(`warning ${minutes}`)}
                                            onRemove={() => {
                                                const next =
                                                    draft.warnings[index + 1] ?? draft.warnings[index - 1];
                                                focusAfterRemoval.current = {
                                                    chip: next === undefined ? undefined : `warning ${next}`,
                                                    field: 'warning',
                                                };
                                                edit({
                                                    warnings: draft.warnings.filter(
                                                        (other) => other !== minutes,
                                                    ),
                                                });
                                            }}
                                        />
                                    </li>
                                ))}
                            </ul>
                            {issue('warnings') && <FieldError>{issue('warnings')}</FieldError>}
                            {warningIssues.map(({ minutes, message }) => (
                                <FieldError key={minutes}>
                                    {t.schedule.issueOf(t.schedule.warningMinutes(minutes), message ?? '')}
                                </FieldError>
                            ))}
                            <div className="flex flex-wrap items-end gap-2">
                                <TextField
                                    ref={addWarningField}
                                    label={t.schedule.addWarning}
                                    type="number"
                                    inputMode="numeric"
                                    min={1}
                                    max={MAX_WARNING_MINUTES}
                                    className="w-56 max-md:flex-1"
                                    value={newWarning}
                                    error={warningProblem ?? undefined}
                                    onChange={(event) => {
                                        setNewWarning(event.target.value);
                                        setWarningProblem(null);
                                    }}
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
                        </fieldset>
                    )}
                </SettingRow>
                <SettingRow id="skipAfterStart" field changed={is('skipAfterStart')}>
                    {(ids) => (
                        <>
                            <input
                                id={ids.control}
                                type="number"
                                inputMode="numeric"
                                min={0}
                                max={720}
                                className={cn(inputClass, 'max-w-32')}
                                aria-invalid={issue('skipAfterStart') ? true : undefined}
                                aria-describedby={
                                    issue('skipAfterStart') ? `${ids.control}-error ${ids.about}` : ids.about
                                }
                                value={Number.isNaN(draft.skipAfterStart) ? '' : draft.skipAfterStart}
                                onChange={(event) =>
                                    edit({
                                        skipAfterStart:
                                            event.target.value === ''
                                                ? Number.NaN
                                                : Number(event.target.value),
                                    })
                                }
                            />
                            {issue('skipAfterStart') && (
                                <FieldError id={`${ids.control}-error`}>{issue('skipAfterStart')}</FieldError>
                            )}
                        </>
                    )}
                </SettingRow>
            </SettingsGroup>
        </fieldset>
    );
}

/** A removable chip: a time or a warning of the settings. */
function ListChip({
    label,
    remove,
    invalid,
    removeRef,
    onRemove,
}: {
    label: string;
    remove: string;
    invalid: boolean;
    removeRef: Ref<HTMLButtonElement>;
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
                ref={removeRef}
                type="button"
                className="grid size-5 place-items-center rounded-pill text-ink-3 hover:bg-hover hover:text-ink focus-visible:outline-2 focus-visible:outline-accent max-md:-my-px max-md:-mr-1 max-md:size-11"
                aria-label={remove}
                onClick={onRemove}
            >
                <X size={12} strokeWidth={2} aria-hidden="true" />
            </button>
        </span>
    );
}
