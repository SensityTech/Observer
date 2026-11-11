import type { Feature } from '../api/generated/Feature';
import type { Me } from '../api/generated/Me';
import { Permission } from '../api/generated/Permission';
import { allows } from '../auth/me';
import type { PermissionName } from '../auth/permissions';
import type { Messages } from '../i18n/en';

/** When a change takes effect, which each row and the save bar say. */
export type Applies = 'now' | 'nextStart' | 'observerStart' | 'withinHour' | 'nextRun' | 'cleanup' | 'file';

export type SettingId =
    | 'dataPath'
    | 'cfgFile'
    | 'build'
    | 'onesync'
    | 'autoStart'
    | 'txadminEvents'
    | 'hangDetection'
    | 'hangTimeout'
    | 'extraArguments'
    | 'scheduleEnabled'
    | 'scheduleMode'
    | 'scheduleTimes'
    | 'scheduleInterval'
    | 'timeZone'
    | 'warnings'
    | 'skipAfterStart'
    | 'keepBuilds'
    | 'autoRollback'
    | 'branches'
    | 'backupsBeforeRestarts'
    | 'backupsTimes'
    | 'backupsDestinations'
    | 'backupsLast'
    | 'refusalMessage'
    | 'tokenMatches'
    | 'allowlistEnabled'
    | 'allowlistMessage'
    | 'maintenanceMessage'
    | 'maintenancePasses'
    | 'maintenanceServerName'
    | 'mapImagery'
    | 'mapImage'
    | 'mapBounds'
    | 'mapTiers'
    | 'mapCadence'
    | 'switchSftp'
    | 'switchChanges'
    | 'switchTools'
    | 'switchDiagnostics'
    | 'switchMetrics'
    | 'switchPlayers'
    | 'switchDrops'
    | 'switchUpdates'
    | 'switchDebug'
    | 'changesEnabled'
    | 'startEvent'
    | 'extensions'
    | 'excluded'
    | 'hidden'
    | 'maxText'
    | 'budget'
    | 'retentionDays'
    | 'hashRate'
    | 'verifyDays'
    | 'sessionDays'
    | 'linkDays'
    | 'addressDays'
    | 'dayDays'
    | 'profileDays'
    | 'playersImport'
    | 'statisticsKept'
    | 'toolAtStart'
    | 'toolDifferences'
    | 'toolEscrowed'
    | 'toolExcluded'
    | 'listen'
    | 'hosts'
    | 'trustedProxies'
    | 'accounts'
    | 'sftpAddress'
    | 'fileGroups'
    | 'filePeople'
    | 'filePipelines'
    | 'password'
    | 'sessions'
    | 'sshKeys';

export type CategoryId =
    | 'launch'
    | 'restarts'
    | 'builds'
    | 'backups'
    | 'bans'
    | 'allowlist'
    | 'maintenance'
    | 'map'
    | 'features'
    | 'data'
    | 'tools'
    | 'panel'
    | 'users'
    | 'files'
    | 'account';

export type CategoryGroup = 'server' | 'observer' | 'people';

/** The owner alone, for what no permission grants; any one of a list of permissions; or anyone signed in. */
type Access = 'owner' | 'everyone' | PermissionName[];

export type Category = {
    id: CategoryId;
    group: CategoryGroup;
    access: Access;
    /** In the order the category shows them; a setting without `applies` is read or acted on, not saved. */
    settings: { id: SettingId; applies?: Applies }[];
};

export const CATEGORIES: readonly Category[] = [
    {
        id: 'launch',
        group: 'server',
        access: 'owner',
        settings: [
            { id: 'dataPath', applies: 'nextStart' },
            { id: 'cfgFile', applies: 'nextStart' },
            { id: 'build' },
            { id: 'onesync', applies: 'nextStart' },
            { id: 'autoStart', applies: 'observerStart' },
            { id: 'txadminEvents', applies: 'nextStart' },
            { id: 'hangDetection', applies: 'now' },
            { id: 'hangTimeout', applies: 'now' },
            { id: 'extraArguments', applies: 'nextStart' },
        ],
    },
    {
        id: 'restarts',
        group: 'server',
        access: ['DashboardView', 'ScheduleManage', 'ServerControl'],
        settings: [
            { id: 'scheduleEnabled', applies: 'now' },
            { id: 'scheduleMode', applies: 'now' },
            { id: 'scheduleTimes', applies: 'now' },
            { id: 'scheduleInterval', applies: 'now' },
            { id: 'timeZone', applies: 'now' },
            { id: 'warnings', applies: 'now' },
            { id: 'skipAfterStart', applies: 'now' },
        ],
    },
    {
        id: 'builds',
        group: 'server',
        access: ['ArtifactsView'],
        settings: [
            { id: 'keepBuilds', applies: 'cleanup' },
            { id: 'autoRollback', applies: 'now' },
            { id: 'branches', applies: 'now' },
        ],
    },
    {
        id: 'backups',
        group: 'server',
        access: ['BackupsView'],
        settings: [
            { id: 'backupsBeforeRestarts' },
            { id: 'backupsTimes' },
            { id: 'backupsDestinations' },
            { id: 'backupsLast' },
        ],
    },
    {
        id: 'bans',
        group: 'server',
        access: ['SettingsView'],
        settings: [
            { id: 'refusalMessage', applies: 'now' },
            { id: 'tokenMatches', applies: 'now' },
        ],
    },
    {
        id: 'allowlist',
        group: 'server',
        access: ['SettingsView'],
        settings: [
            { id: 'allowlistEnabled', applies: 'now' },
            { id: 'allowlistMessage', applies: 'now' },
        ],
    },
    {
        id: 'maintenance',
        group: 'server',
        access: ['SettingsView'],
        settings: [
            { id: 'maintenanceMessage', applies: 'now' },
            { id: 'maintenancePasses', applies: 'now' },
            { id: 'maintenanceServerName', applies: 'now' },
        ],
    },
    {
        id: 'map',
        group: 'server',
        access: ['SettingsView', 'PlayersMap'],
        settings: [
            { id: 'mapImagery', applies: 'now' },
            { id: 'mapImage' },
            { id: 'mapBounds', applies: 'now' },
            { id: 'mapTiers', applies: 'now' },
            { id: 'mapCadence', applies: 'now' },
        ],
    },
    {
        id: 'features',
        group: 'observer',
        access: ['SettingsView'],
        settings: [
            { id: 'switchSftp' },
            { id: 'switchChanges' },
            { id: 'switchTools' },
            { id: 'switchDiagnostics' },
            { id: 'switchMetrics' },
            { id: 'switchPlayers' },
            { id: 'switchDrops' },
            { id: 'switchUpdates' },
            { id: 'switchDebug' },
        ],
    },
    {
        id: 'data',
        group: 'observer',
        access: 'owner',
        settings: [
            { id: 'changesEnabled', applies: 'nextStart' },
            { id: 'startEvent', applies: 'nextStart' },
            { id: 'extensions', applies: 'nextStart' },
            { id: 'excluded', applies: 'nextStart' },
            { id: 'hidden', applies: 'nextStart' },
            { id: 'maxText', applies: 'nextStart' },
            { id: 'budget', applies: 'withinHour' },
            { id: 'retentionDays', applies: 'withinHour' },
            { id: 'hashRate', applies: 'nextStart' },
            { id: 'verifyDays', applies: 'nextStart' },
            { id: 'sessionDays', applies: 'withinHour' },
            { id: 'linkDays', applies: 'withinHour' },
            { id: 'addressDays', applies: 'withinHour' },
            { id: 'dayDays', applies: 'withinHour' },
            { id: 'profileDays', applies: 'withinHour' },
            { id: 'playersImport' },
            { id: 'statisticsKept' },
        ],
    },
    {
        id: 'tools',
        group: 'observer',
        access: 'owner',
        settings: [
            { id: 'toolAtStart', applies: 'nextRun' },
            { id: 'toolDifferences', applies: 'nextRun' },
            { id: 'toolEscrowed', applies: 'nextRun' },
            { id: 'toolExcluded', applies: 'nextRun' },
        ],
    },
    {
        id: 'panel',
        group: 'observer',
        access: 'owner',
        settings: [
            { id: 'listen', applies: 'file' },
            { id: 'hosts', applies: 'file' },
            { id: 'trustedProxies', applies: 'file' },
        ],
    },
    {
        id: 'users',
        group: 'people',
        access: ['UsersManage'],
        settings: [{ id: 'accounts' }],
    },
    {
        id: 'files',
        group: 'people',
        access: ['FilesManage'],
        settings: [
            { id: 'sftpAddress' },
            { id: 'fileGroups' },
            { id: 'filePeople' },
            { id: 'filePipelines' },
        ],
    },
    {
        id: 'account',
        group: 'people',
        access: 'everyone',
        settings: [{ id: 'password' }, { id: 'sessions' }, { id: 'sshKeys' }],
    },
];

export const GROUPS: readonly CategoryGroup[] = ['server', 'observer', 'people'];

/** The switch of each optional feature; when a switch applies, the API says, feature by feature. */
export const SWITCH_OF = {
    sftp: 'switchSftp',
    changes: 'switchChanges',
    tools: 'switchTools',
    diagnostics: 'switchDiagnostics',
    metrics: 'switchMetrics',
    players: 'switchPlayers',
    drops: 'switchDrops',
    updates: 'switchUpdates',
    debug: 'switchDebug',
} satisfies Record<Feature, SettingId>;

/** When each setting applies, found by its id. */
export const APPLIES = new Map(
    CATEGORIES.flatMap(({ settings }) => settings.map(({ id, applies }) => [id, applies] as const)),
);

/** The address of a setting's row: its category's page, scrolled to it. */
export const anchorOf = (id: SettingId) => `setting-${id}`;

export const opens = (me: Me, { access }: Category) =>
    access === 'everyone'
        ? true
        : access === 'owner'
          ? me.isOwner
          : access.some((name) => allows(me, Permission[name]));

export const isCategory = (id: string): id is CategoryId => CATEGORIES.some((category) => category.id === id);

/** Where `/settings` leads: the first category the user opens, their own account at worst. */
export const firstCategory = (me: Me): CategoryId =>
    CATEGORIES.find((category) => opens(me, category))?.id ?? 'account';

export type Match = { category: Category; id: SettingId; applies?: Applies };

/** Lower case without accents, so that `Onesync` or `fingerprint` finds what is written otherwise. */
const folded = (text: string) =>
    text
        .normalize('NFD')
        .replace(/\p{Diacritic}/gu, '')
        .toLowerCase();

/**
 * The settings of the categories the user opens whose name, explanation or category holds every
 * word of the query, in the order the categories show them.
 */
export const search = (t: Messages, me: Me, query: string): Match[] => {
    const words = folded(query).split(/\s+/).filter(Boolean);
    if (words.length === 0) {
        return [];
    }
    return CATEGORIES.filter((category) => opens(me, category)).flatMap((category) =>
        category.settings
            .filter(({ id }) => {
                const { label, about } = t.settings.items[id];
                const text = folded(`${label} ${about} ${t.settings.categories[category.id].name}`);
                return words.every((word) => text.includes(word));
            })
            .map(({ id, applies }) => ({ category, id, applies })),
    );
};
