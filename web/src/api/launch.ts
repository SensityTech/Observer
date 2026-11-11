import { queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { LaunchChange } from './generated/LaunchChange';
import type { LaunchField } from './generated/LaunchField';
import type { LaunchIssue } from './generated/LaunchIssue';
import type { LaunchProblem } from './generated/LaunchProblem';
import type { LaunchSettings } from './generated/LaunchSettings';
import type { StoredLaunch } from './generated/StoredLaunch';

const PATH = '/servers/default/launch';

/** The stored settings and their version, or null until the owner first saves them. */
export const launchQuery = queryOptions({
    queryKey: ['launch'],
    queryFn: ({ signal }) => getJson<StoredLaunch | null>(PATH, signal),
});

/** Refused with `conflict` when the settings changed since the version the change was made from. */
export const saveLaunch = (change: LaunchChange) => send<StoredLaunch>('PUT', PATH, change);

/** What the form starts from before anything is saved. */
export const DEFAULT_LAUNCH: LaunchSettings = {
    dataPath: '',
    cfgFile: 'server.cfg',
    onesync: 'on',
    autoStart: false,
    txadminEvents: false,
    extraArguments: [],
    hangDetection: true,
    hangTimeoutSeconds: 360,
    stopHoldSeconds: 60,
};

const FIELDS = {
    dataPath: true,
    cfgFile: true,
    extraArguments: true,
    argumentName: true,
    argumentValue: true,
    hangTimeoutSeconds: true,
    stopHoldSeconds: true,
} satisfies Record<LaunchField, true>;

const PROBLEMS = {
    required: true,
    tooLong: true,
    notAbsolute: true,
    networkPath: true,
    devicePath: true,
    controlCharacter: true,
    fxdk: true,
    invalidCfgFile: true,
    optionLike: true,
    invalidName: true,
    forbiddenCharacter: true,
    quoteOrSemicolon: true,
    reserved: true,
    secret: true,
    duplicate: true,
    tooMany: true,
    outOfRange: true,
} satisfies Record<LaunchProblem, true>;

const isIssue = (value: unknown): value is LaunchIssue =>
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

/** The problem a refusal names, when it is one the panel knows. */
export const launchProblem = (value: unknown): LaunchProblem | null =>
    typeof value === 'string' && Object.hasOwn(PROBLEMS, value) ? (value as LaunchProblem) : null;

/** The issues an `invalid_launch_settings` refusal lists, leaving out any the panel cannot place. */
export const launchIssues = (params: Record<string, unknown>): LaunchIssue[] =>
    Array.isArray(params.issues) ? params.issues.filter(isIssue) : [];

/** The settings that differ, the list of extra arguments counting as one. */
export const changedSettings = (saved: LaunchSettings, draft: LaunchSettings) =>
    (Object.keys(draft) as (keyof LaunchSettings)[]).filter(
        (key) => JSON.stringify(saved[key]) !== JSON.stringify(draft[key]),
    );

/** How many settings differ, the list of extra arguments counting as one. */
export const countChanges = (saved: LaunchSettings, draft: LaunchSettings) =>
    changedSettings(saved, draft).length;

/** The settings a running server takes at once. */
export const HANG_SETTINGS: ReadonlySet<keyof LaunchSettings> = new Set([
    'hangDetection',
    'hangTimeoutSeconds',
]);

/** The seconds a stop waits for the resources that hold it, at the least and at the most. */
export const STOP_HOLD_RANGE = { min: 5, max: 600 } as const;

/** The draft as it is sent: names and paths lose the spaces a paste leaves around them. */
export const cleaned = (draft: LaunchSettings): LaunchSettings => ({
    ...draft,
    dataPath: draft.dataPath.trim(),
    cfgFile: draft.cfgFile.trim(),
    extraArguments: draft.extraArguments.map(({ name, value }) => ({ name: name.trim(), value })),
});
