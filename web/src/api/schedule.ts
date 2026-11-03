import { send } from './client';
import type { OneOffRequest } from './generated/OneOffRequest';
import type { ScheduleChange } from './generated/ScheduleChange';
import type { ScheduleField } from './generated/ScheduleField';
import type { ScheduleIssue } from './generated/ScheduleIssue';
import type { ScheduleProblem } from './generated/ScheduleProblem';
import type { ScheduleSettings } from './generated/ScheduleSettings';
import type { ScheduleStatus } from './generated/ScheduleStatus';

const PATH = '/servers/default/schedule';

// Every route answers the whole status, which the `server.schedule` topic carries too.

/** Refused with `conflict` when the settings changed since the version the change was made from. */
export const saveSchedule = (change: ScheduleChange) => send<ScheduleStatus>('PUT', PATH, change);
export const skipNext = () => send<ScheduleStatus>('POST', `${PATH}/skip`);
export const restoreSkipped = () => send<ScheduleStatus>('DELETE', `${PATH}/skip`);
export const delayNext = (minutes: number) => send<ScheduleStatus>('POST', `${PATH}/delay`, { minutes });
export const restartOnce = (request: OneOffRequest) => send<ScheduleStatus>('PUT', `${PATH}/once`, request);
export const cancelOnce = () => send<ScheduleStatus>('DELETE', `${PATH}/once`);

/** txAdmin's warnings, which Observer starts from. */
export const DEFAULT_WARNINGS = [30, 15, 10, 5, 4, 3, 2, 1];

const FIELDS = {
    times: true,
    time: true,
    intervalMinutes: true,
    warning: true,
    warnings: true,
    skipAfterStart: true,
    timeZone: true,
} satisfies Record<ScheduleField, true>;

const PROBLEMS = {
    required: true,
    tooMany: true,
    invalidTime: true,
    duplicate: true,
    tooClose: true,
    outOfRange: true,
    unknownTimeZone: true,
} satisfies Record<ScheduleProblem, true>;

const isIssue = (value: unknown): value is ScheduleIssue =>
    typeof value === 'object' &&
    value !== null &&
    'field' in value &&
    typeof value.field === 'string' &&
    Object.hasOwn(FIELDS, value.field) &&
    'problem' in value &&
    typeof value.problem === 'string' &&
    Object.hasOwn(PROBLEMS, value.problem) &&
    'index' in value &&
    (value.index === null || Number.isSafeInteger(value.index));

/** The issues an `invalid_schedule` refusal lists, leaving out any the panel cannot place. */
export const scheduleIssues = (params: Record<string, unknown>): ScheduleIssue[] =>
    Array.isArray(params.issues) ? params.issues.filter(isIssue) : [];

/** The settings that differ, each list counting as one. */
export const changedSettings = (saved: ScheduleSettings, draft: ScheduleSettings) =>
    (Object.keys(draft) as (keyof ScheduleSettings)[]).filter(
        (key) => JSON.stringify(saved[key]) !== JSON.stringify(draft[key]),
    );

/** How many settings differ, each list counting as one. */
export const countChanges = (saved: ScheduleSettings, draft: ScheduleSettings) =>
    changedSettings(saved, draft).length;
