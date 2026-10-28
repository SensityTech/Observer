import { queryOptions } from '@tanstack/react-query';

import { getJson, send } from './client';
import type { LaunchField } from './generated/LaunchField';
import type { LaunchIssue } from './generated/LaunchIssue';
import type { LaunchProblem } from './generated/LaunchProblem';
import type { LaunchSettings } from './generated/LaunchSettings';

const PATH = '/servers/default/launch';

/** The stored settings, or null until the owner first saves them. */
export const launchQuery = queryOptions({
    queryKey: ['launch'],
    queryFn: ({ signal }) => getJson<LaunchSettings | null>(PATH, signal),
});

export const saveLaunch = (settings: LaunchSettings) => send<LaunchSettings>('PUT', PATH, settings);

/** What the form starts from before anything is saved. */
export const DEFAULT_LAUNCH: LaunchSettings = {
    buildPath: '',
    dataPath: '',
    cfgFile: 'server.cfg',
    onesync: 'on',
    autoStart: false,
    txadminEvents: false,
    extraArguments: [],
};

const FIELDS = {
    buildPath: true,
    dataPath: true,
    cfgFile: true,
    extraArguments: true,
    argumentName: true,
    argumentValue: true,
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
    reserved: true,
    secret: true,
    duplicate: true,
    tooMany: true,
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

/** The issues an `invalid_launch_settings` refusal lists, leaving out any the panel cannot place. */
export const launchIssues = (params: Record<string, unknown>): LaunchIssue[] =>
    Array.isArray(params.issues) ? params.issues.filter(isIssue) : [];

/** How many settings differ, the list of extra arguments counting as one. */
export const countChanges = (saved: LaunchSettings, draft: LaunchSettings) =>
    (Object.keys(draft) as (keyof LaunchSettings)[]).filter(
        (key) => JSON.stringify(saved[key]) !== JSON.stringify(draft[key]),
    ).length;

/** The draft as it is sent: names and paths lose the spaces a paste leaves around them. */
export const cleaned = (draft: LaunchSettings): LaunchSettings => ({
    ...draft,
    buildPath: draft.buildPath.trim(),
    dataPath: draft.dataPath.trim(),
    cfgFile: draft.cfgFile.trim(),
    extraArguments: draft.extraArguments.map(({ name, value }) => ({ name: name.trim(), value })),
});
