// The source catalog. Every other locale is typed against it, so a key missing there fails the type
// check. Messages that take values are functions; they format with the helpers bound to this locale.

import type { ReactNode } from 'react';

import type { ActorKind } from '../api/generated/ActorKind';
import type { ApplyMode } from '../api/generated/ApplyMode';
import type { ArchetypeType } from '../api/generated/ArchetypeType';
import type { ArgumentChangeKind } from '../api/generated/ArgumentChangeKind';
import type { ArtifactSettingsProblem } from '../api/generated/ArtifactSettingsProblem';
import type { AuditAction } from '../api/generated/AuditAction';
import type { BuildPlatform } from '../api/generated/BuildPlatform';
import type { CfgFileProblem } from '../api/generated/CfgFileProblem';
import type { CfgFileState } from '../api/generated/CfgFileState';
import type { ChartCategory } from '../api/generated/ChartCategory';
import type { CommandRefusal } from '../api/generated/CommandRefusal';
import type { Contest } from '../api/generated/Contest';
import type { CrashForm } from '../api/generated/CrashForm';
import type { Difference } from '../api/generated/Difference';
import type { Differences } from '../api/generated/Differences';
import type { DropCategory } from '../api/generated/DropCategory';
import type { DropKind } from '../api/generated/DropKind';
import type { DropsRange } from '../api/generated/DropsRange';
import type { EndpointProtocol } from '../api/generated/EndpointProtocol';
import type { EnvironmentFact } from '../api/generated/EnvironmentFact';
import type { ErrorCode } from '../api/generated/ErrorCode';
import type { EscrowPolicy } from '../api/generated/EscrowPolicy';
import type { Feature } from '../api/generated/Feature';
import type { FileChangeKind } from '../api/generated/FileChangeKind';
import type { FileFlag } from '../api/generated/FileFlag';
import type { FileFormat } from '../api/generated/FileFormat';
import type { FileKind } from '../api/generated/FileKind';
import type { FileProblem } from '../api/generated/FileProblem';
import type { FindingKind } from '../api/generated/FindingKind';
import type { FindingParams } from '../api/generated/FindingParams';
import type { FolderProblem } from '../api/generated/FolderProblem';
import type { Game } from '../api/generated/Game';
import type { HealthProblem } from '../api/generated/HealthProblem';
import type { IdentifierKind } from '../api/generated/IdentifierKind';
import type { InstallFailure } from '../api/generated/InstallFailure';
import type { JobKind } from '../api/generated/JobKind';
import type { JobStep } from '../api/generated/JobStep';
import type { LaunchProblem } from '../api/generated/LaunchProblem';
import type { LeaveReason } from '../api/generated/LeaveReason';
import type { LicenceState } from '../api/generated/LicenceState';
import type { LineKind } from '../api/generated/LineKind';
import type { LiveChangeKind } from '../api/generated/LiveChangeKind';
import type { ModelInfoKind } from '../api/generated/ModelInfoKind';
import type { NoticeReason } from '../api/generated/NoticeReason';
import type { NotWritable } from '../api/generated/NotWritable';
import type { OneSync } from '../api/generated/OneSync';
import type { Outcome } from '../api/generated/Outcome';
import type { PartialReason } from '../api/generated/PartialReason';
import type { PermissionArea } from '../api/generated/PermissionArea';
import type { PlayerSort } from '../api/generated/PlayerSort';
import type { PlayersFileProblem } from '../api/generated/PlayersFileProblem';
import type { Protection } from '../api/generated/Protection';
import type { Range } from '../api/generated/Range';
import type { ResourceChangeKind } from '../api/generated/ResourceChangeKind';
import type { ScheduleMode } from '../api/generated/ScheduleMode';
import type { ScheduleProblem } from '../api/generated/ScheduleProblem';
import type { ServerState } from '../api/generated/ServerState';
import type { Severity } from '../api/generated/Severity';
import type { SnapshotState } from '../api/generated/SnapshotState';
import type { SpawnFailure } from '../api/generated/SpawnFailure';
import type { StartCause } from '../api/generated/StartCause';
import type { StopSource } from '../api/generated/StopSource';
import type { SupportLevel } from '../api/generated/SupportLevel';
import type { TileKind } from '../api/generated/TileKind';
import type { ToolCloseReason } from '../api/generated/ToolCloseReason';
import type { ToolFileFormat } from '../api/generated/ToolFileFormat';
import type { ToolMode } from '../api/generated/ToolMode';
import type { ToolNoteKind } from '../api/generated/ToolNoteKind';
import type { ToolRunMode } from '../api/generated/ToolRunMode';
import type { ToolRunOutcome } from '../api/generated/ToolRunOutcome';
import type { ToolTrigger } from '../api/generated/ToolTrigger';
import type { TrendVerdict } from '../api/generated/TrendVerdict';
import { parsePermissions, permissionKeys, type PermissionKey } from '../auth/permissions';
import type { InvisibleKind } from '../changes/invisible';
import type { LinkState } from '../live/realtime';
import type { LaunchStep, SetupStep } from '../setup/state';
import { formatBytes, formatDuration, formatNumber, plural } from './format';
import { numberParam, stringParam } from './params';

type Params = Record<string, unknown>;
export type ErrorMessage = string | ((params: Params) => string);

const count = (value: number, forms: { one: string; other: string }) =>
    `${formatNumber('en', value)} ${plural('en', value, forms)}`;

const list = (items: string[]) => new Intl.ListFormat('en', { type: 'conjunction' }).format(items);

const ORDINAL_SUFFIXES: Partial<Record<Intl.LDMLPluralRule, string>> = { one: 'st', two: 'nd', few: 'rd' };

/** `1st`, `2nd`, `3rd`, `11th`. */
const ordinal = (value: number) =>
    `${formatNumber('en', value)}${ORDINAL_SUFFIXES[new Intl.PluralRules('en', { type: 'ordinal' }).select(value)] ?? 'th'}`;

const permissions = {
    dashboardView: { name: 'View the dashboard', description: 'Server state, statistics and charts.' },
    consoleView: {
        name: 'View the console',
        description: 'The live FXServer output, which can show player addresses.',
    },
    consoleWrite: {
        name: 'Send console commands',
        description: 'Any command FXServer accepts, which reaches everything the server can.',
    },
    serverControl: {
        name: 'Start and stop the server',
        description: 'Start, stop and restart FXServer, now or once at a set time.',
    },
    scheduleManage: {
        name: 'Manage the schedule',
        description: 'Add, change and remove scheduled restarts.',
    },
    artifactsView: { name: 'View builds', description: 'The FXServer builds installed and available.' },
    artifactsManage: {
        name: 'Manage builds',
        description: 'Download, install and switch FXServer builds, which changes what the host runs.',
    },
    usersManage: {
        name: 'Manage users',
        description: 'Create accounts and grant them the permissions you hold yourself.',
    },
    auditView: {
        name: 'View the audit log',
        description: 'Every action taken in Observer, by whom and from where.',
    },
    settingsView: { name: 'View settings', description: "Observer's configuration." },
    settingsManage: { name: 'Change settings', description: "Change Observer's configuration." },
    logsDownload: { name: 'Download logs', description: "Observer's and FXServer's log files." },
    diagnosticsView: {
        name: 'View diagnostics',
        description:
            'Processes FXServer starts and their command lines, its connections, crash and hang analyses.',
    },
    filesManage: {
        name: 'Manage file access',
        description:
            "Give accounts Read and Assets groups over the server's files, take access away and end sessions.",
    },
    playersView: {
        name: 'View players',
        description: "Who is connected, and each player's names, licence, play time and disconnections.",
    },
    playerIdentifiersView: {
        name: 'View player identifiers',
        description:
            'Every identifier a player showed, hardware tokens included, and the other licences they link to.',
    },
    dropsView: {
        name: 'View player drops',
        description: 'Drop rates, categories, crash signatures and kicking resources, without any player.',
    },
    changesView: {
        name: 'View changes',
        description:
            'What each start of the server ran from: resources, versions, files, sizes and who changed them.',
    },
    changesContent: {
        name: 'Read changed files',
        description:
            "Line diffs and whole files of the server's resources, secrets hidden. The server's code is its most valuable asset.",
    },
} satisfies Record<PermissionKey, { name: string; description: string }>;

const permissionNames = (hex: string) =>
    list(permissionKeys(parsePermissions(hex)).map((key) => permissions[key].name));

const isPermissionKey = (key: string): key is PermissionKey => Object.hasOwn(permissions, key);

const features = {
    sftp: 'File access over SFTP',
    changes: 'File changes',
    tools: 'Server tools',
    diagnostics: 'Diagnostics',
    metrics: 'Statistics',
    players: 'Players',
    drops: 'Player drops',
} satisfies Record<Feature, string>;

const isFeature = (key: string): key is Feature => Object.hasOwn(features, key);

const commandRefusals = {
    empty: 'Type a command first.',
    tooLong: 'This command is too long: 1,024 bytes at most.',
    controlCharacter: 'Remove the control characters from the command.',
    reserved: 'observer_cmd is reserved to Observer: no account can send it.',
} satisfies Record<CommandRefusal, string>;

const isCommandRefusal = (key: string): key is CommandRefusal => Object.hasOwn(commandRefusals, key);

// What failed, without the folders it names: those are the owner's, and the console has them.
const spawnFailures = {
    notConfigured: 'no launch settings are saved.',
    invalidSettings: 'this host refuses its launch settings.',
    notABuild: 'the build folder holds no FXServer program.',
    missingData: 'the server data folder does not exist.',
    cfgUnreadable: 'the server cfg cannot be read.',
    bridge: 'the observer bridge could not be written into the build.',
    spawn: 'the system refused to start the process.',
} satisfies Record<SpawnFailure, string>;

const isSpawnFailure = (key: string): key is SpawnFailure => Object.hasOwn(spawnFailures, key);

const healthProblems = {
    httpLate: 'its HTTP endpoint answers late',
    heartbeatLate: 'the bridge heartbeat is late',
    mainThreadStalled: 'its main thread stalled',
    probeUnauthorized: 'its performance probe refuses Observer',
    watchdogBark: 'its watchdog reported a stuck thread',
} satisfies Record<HealthProblem, string>;

const playersFileProblems = {
    missing: 'Nothing is at this path on the host.',
    notAFile: 'This is not a plain file: a folder, a link or a device.',
    tooLarge: 'This file is over 512 MB, more than a players database holds.',
    unreadable: 'Observer cannot read this file: check its permissions.',
    notPlayersDatabase: "This file is not txAdmin's players database.",
    unsupportedVersion:
        'txAdmin wrote this file in a format Observer does not read. Open it once with a current txAdmin, then import it again.',
    notLocal: 'Give the full path of a file on this host, not a network share or a relative path.',
} satisfies Record<PlayersFileProblem, string>;

const isPlayersFileProblem = (key: string): key is PlayersFileProblem =>
    Object.hasOwn(playersFileProblems, key);

const ranges = {
    '1h': '1h',
    '6h': '6h',
    '24h': '24h',
    '7d': '7d',
    '30d': '30d',
} satisfies Record<Range, string>;

/** An endpoint a finding names: `the TCP endpoint 0.0.0.0:30120`. */
const endpointOf = ({ protocol, address }: FindingParams) => {
    const name = protocol === 'udp' ? 'UDP' : 'TCP';
    return address === undefined ? `a ${name} endpoint` : `the ${name} endpoint ${address}`;
};

// What a finding of the cfg says; the panel shows its file and line beside it.
const cfgFindings = {
    noEndpoint: () =>
        'Adds no endpoint: FXServer would listen nowhere, so neither players nor Observer could reach it.',
    endpointInvalid: (params) => `Adds ${endpointOf(params)}, which is not one ip:port address.`,
    endpointOnPanel: (params) =>
        `Adds ${endpointOf(params)} on the panel's port: FXServer and Observer cannot both listen there.`,
    endpointUnpaired: (params) =>
        `Adds ${endpointOf(params)} without its ${params.protocol === 'udp' ? 'TCP' : 'UDP'} twin: players cannot connect.`,
    endpointDuplicate: (params) => `Adds ${endpointOf(params)} again.`,
    endpointReserved: (params) =>
        `Adds ${endpointOf(params)} among txAdmin's ports, 40120 to 40150: fine under Observer, but a txAdmin running on this host takes them.`,
    portInUse: (params) =>
        `Adds ${endpointOf(params)} on a port FXServer cannot listen on now: another program holds it, or the system reserves it.`,
    startsMonitor: (params) =>
        `Starts txAdmin's ${params.name ?? ''} resource, which only runs under txAdmin.`,
    setsServerMode: () => "Sets txAdminServerMode: FXServer starts txAdmin's monitor resource by itself.",
    setsMonitorMode: () => 'Sets monitorMode, which only txAdmin reads.',
    setsObserverConvar: (params) =>
        `Sets ${params.name ?? ''}, which Observer sets at each launch: Observer's value wins.`,
    plusCommand: (params) => `Writes ${params.name ?? ''}, which FXServer only reads on its command line.`,
    licenceMissing: () => 'Sets no licence key: FXServer stops at start.',
    licenceUnusable: (params) =>
        params.licence === 'placeholder'
            ? 'Sets a placeholder for the licence key: FXServer stops at start.'
            : 'Sets a licence key that does not look like one Cfx.re gives.',
    maxClientsInvalid: (params) => `Sets sv_maxclients to ${params.value ?? ''}, outside 1 to 2048.`,
    maxClientsNeedOneSync: (params) =>
        `Allows ${count(params.count ?? 0, { one: 'player', other: 'players' })}: more than 32 needs OneSync on.`,
    gameInvalid: (params) => `Sets gamename to ${params.value ?? ''}, neither gta5 nor rdr3.`,
    gameTwice: (params) => `Sets gamename again, to ${params.value ?? ''}.`,
    resourceMissing: (params) =>
        `Starts ${params.name ?? ''}, which is no resource of the server folder or of the build.`,
    resourceMissingMore: (params) =>
        `Starts ${count(params.count ?? 0, { one: 'more resource that is', other: 'more resources that are' })} missing.`,
    resourceNamedObserver: () =>
        'Takes the name of the observer bridge, which Observer adds at each launch: rename this resource.',
    execMissing: (params) => `Runs ${params.target ?? ''}, which cannot be read.`,
    execOutside: (params) =>
        `Runs ${params.target ?? ''}, outside the server folder: FXServer runs it, Observer did not check it.`,
    execResource: (params) =>
        `Runs ${params.target ?? ''} from a resource: FXServer runs it, Observer did not check it.`,
    execTooDeep: (params) =>
        `Runs ${params.target ?? ''}, more than 5 files deep: FXServer runs it, Observer did not check it.`,
    execRepeated: (params) =>
        `Runs ${params.target ?? ''} again: FXServer runs it each time, and a loop never ends.`,
    editedByTxAdmin: (params) =>
        `Holds ${count(params.count ?? 0, { one: 'line', other: 'lines' })} txAdmin commented out: Observer leaves them as they are.`,
} satisfies Record<FindingKind, (params: FindingParams) => string>;

/** What else could put another file in a `.ytyp`'s place. */
const contestText = (by: Contest) => {
    switch (by.kind) {
        case 'sameName':
            return `${by.path} in ${by.resource} has the same name and may be loaded in its place`;
        case 'sibling':
            return `${by.path}, of the same name in the same resource, may be sent in its place`;
        case 'requested':
            return `${by.resource} requests it with DLC_ITYP_REQUEST`;
        case 'manifest':
            return `${by.path} in ${by.resource} lists it`;
    }
};

const notWritableText = (why: NotWritable) => {
    switch (why.kind) {
        case 'layout':
            return `its layout is not one the fix is proven on (${why.detail})`;
        case 'readOnly':
            return 'it is read-only';
        case 'linked':
            return 'it is reached through a link';
        case 'refused':
            return 'Observer never changes it';
    }
};

/** Why the duplicate archetypes tool leaves a group, naming the file or resource in question. */
const leaveReasonText = (reason: LeaveReason) => {
    switch (reason.kind) {
        case 'definitionsDiffer':
            return 'The copies differ, and only identical copies are fixed.';
        case 'severalResources':
            return 'The copies are in several resources, which can start and stop apart.';
        case 'escrowed':
            return 'A copy is in a resource protected by the asset escrow: pass its files and entries on to its author.';
        case 'interior':
            return 'A copy defines an interior (an MLO), whose removal could take the whole interior away.';
        case 'mixedSources':
            return 'A .ytyp copy meets a .meta one.';
        case 'contested':
            return `${reason.file}: ${contestText(reason.by)}.`;
        case 'manifestDependent':
            return `${reason.file} is listed by ${reason.manifest}, which may load it only with the maps that need it.`;
        case 'notWritable':
            return `${reason.file} cannot be changed: ${notWritableText(reason.why)}.`;
        case 'excluded':
            return `${reason.resource} is excluded in the settings.`;
    }
};

const fileFormats = {
    rsc7: 'RSC7',
    escrowed: 'escrowed',
    pso: 'PSO',
    rbf: 'RBF',
    xml: 'XML',
    empty: 'empty',
    unknown: 'unknown',
} satisfies Record<FileFormat, string>;

const fileProblemText = (problem: FileProblem) => {
    switch (problem.kind) {
        case 'format':
            if (problem.found === 'escrowed') {
                return 'Encrypted by the asset escrow.';
            }
            return problem.found === 'empty'
                ? 'The file is empty.'
                : `Its format is not one the tool reads: ${fileFormats[problem.found]}.`;
        case 'invalid':
            return `Not valid: ${problem.error}`;
        case 'io':
            return `Could not be read: ${problem.error}`;
    }
};

export const en = {
    app: {
        name: 'Observer',
        skipToContent: 'Skip to content',
        plainHttp: {
            title: 'Unencrypted connection.',
            body: 'Passwords and console commands travel in clear text. Put Observer behind an HTTPS proxy or reach it over a VPN.',
        },
    },
    common: {
        cancel: 'Cancel',
        close: 'Close',
        copy: 'Copy',
        copied: 'Copied',
        copyBlocked: 'Copying is blocked here: the text is selected, copy it with your keyboard.',
        copyBlockedValue: (value: string) => `Copying is blocked here. The full value is ${value}.`,
        showPassword: 'Show the password',
        loading: 'Loading',
        retry: 'Try again',
        reload: 'Reload',
        optional: 'optional',
    },
    nav: {
        label: 'Sidebar',
        sections: 'Sections',
        open: 'Open the menu',
        close: 'Close the menu',
        overview: 'Overview',
        console: 'Console',
        administration: 'Administration',
        operations: 'Operations',
        schedule: 'Schedule',
        artifacts: 'Artifacts',
        server: 'Server',
        users: 'Users',
        audit: 'Audit log',
        files: 'Files',
        diagnostics: 'Diagnostics',
        players: 'Players',
        drops: 'Drops',
        changes: 'Changes',
        tools: 'Tools',
        alertCount: (value: number) => count(value, { one: 'alert', other: 'alerts' }),
    },
    theme: {
        label: 'Theme',
        dark: 'Dark',
        light: 'Light',
        switchToDark: 'Switch to dark theme',
        switchToLight: 'Switch to light theme',
    },
    language: {
        current: (language: string) => `Language: ${language}`,
        menu: 'Languages',
    },
    status: {
        label: 'Server status',
        state: 'State',
        build: 'Build',
        players: 'Players',
        playerCount: (online: string, max: string | null) => (max === null ? online : `${online} / ${max}`),
        unknown: 'n/a',
        upFor: (uptime: string) => `up ${uptime}`,
        nextRestart: 'Next restart',
        none: 'None',
        inTime: (left: string) => `in ${left}`,
        probation: 'probation',
        nextBuild: (build: string) => `next ${build}`,
        updates: 'Updates',
        link: {
            connecting: 'Connecting',
            live: 'Live',
            offline: 'Offline',
            // Only another Observer version or an ended session refuses the link: a reload settles both.
            refused: 'Reload needed',
        } satisfies Record<LinkState, string>,
    },
    account: {
        menu: (username: string) => `Account menu for ${username}`,
        owner: 'Owner',
        permissions: (value: number) => count(value, { one: 'permission', other: 'permissions' }),
        myAccount: 'My account',
        signOut: 'Sign out',
        about: 'About Observer',
    },
    overview: {
        title: 'Overview',
        gloss: 'FXServer and Observer, right now.',
        stateTitle: 'FXServer',
        started: (time: string, uptime: string) => `Started ${time} · up ${uptime}`,
        serverLoading: 'Reading the state of FXServer',
        serverUnknown: 'The state of FXServer is not known while the live link is down.',
        serverForbidden: 'Your account cannot see the state of FXServer.',
        build: (build: string) => `Build ${build}`,
        players: (players: string) => `${players} players`,
        notConfigured: 'FXServer has no launch settings yet.',
        notConfiguredOwner: 'The server setup points Observer at your server folder and its cfg.',
        notConfiguredOther: 'The owner saves them in the server setup.',
        invalid: 'This host refuses the stored launch settings.',
        invalidOwner: 'Fix them on the Server page before the next start.',
        invalidOther: 'The owner fixes them on the Server page.',
        noBuild: 'No build is selected yet.',
        noBuildHint: 'Install or import one on the Artifacts page.',
        openServer: 'Open the Server page',
        openSetup: 'Set up the server',
        openArtifacts: 'Open the Artifacts page',
        buildProbation: (build: string, until: string) => `Build ${build}, on probation until ${until}`,
        buildProbationWaiting: (build: string) => `Build ${build}, on probation`,
        buildBack: (build: string, from: string) => `Build ${build}, back from ${from}`,
        buildNextStart: (build: string, next: string) => `Build ${build}, ${next} at the next start`,
        buildPending: (build: string, pending: string) =>
            `Build ${build}, ${pending} at the next scheduled restart`,
        buildPassed: (build: string, at: string) => `Build ${build}, probation passed ${at}`,
        utcClock: (clock: string) => `${clock} UTC`,
        resources: (started: number) => `${count(started, { one: 'resource', other: 'resources' })} started`,
        nextRestart: (time: string, left: string) => `Next restart ${time} (in ${left})`,
    },
    artifacts: {
        title: 'Artifacts',
        gloss: 'The FXServer builds Observer installs and runs.',
        zone: (times: string) => `${times} · dates in UTC`,
        utcTimes: 'Times in UTC',
        loading: 'Reading the builds',
        unknown: 'The builds are not known while the live link is down.',
        outOfDate: 'The live link is down: what follows may be out of date.',
        notices: {
            rolledBack: (to: string, from: string, reason: NoticeReason) =>
                `FXServer went back to build ${to}: ${from} ${reason === 'hung' ? 'stopped answering' : 'crashed'} during its probation.`,
            probationFailed: (build: string, why: NoticeReason, other: string) =>
                why === 'replaced'
                    ? `Build ${build} failed its probation. Build ${other}, chosen since, starts in its place.`
                    : `Build ${build} failed its probation and FXServer kept it: ${
                          {
                              disabled: 'automatic return is off.',
                              noHealthyBuild: 'no healthy build to go back to.',
                              broken: 'the last healthy build is broken.',
                              kvp: 'going back would cross build 6449.',
                              crashed: 'it crashed.',
                              hung: 'it stopped answering.',
                              notInstalled: 'the last healthy build is gone.',
                          }[why]
                      }`,
            pendingDropped: (build: string, reason: NoticeReason) =>
                `Build ${build} was not applied at the scheduled restart: ${reason === 'notInstalled' ? 'it was deleted.' : 'it is broken.'}`,
            legacy: 'FXServer ran from a folder of yours until this version. Observer now runs builds from its data folder: import it to keep using it.',
            importLegacy: 'Import…',
            stale: (source: string, since: string) =>
                `The catalog is not complete: ${source} did not answer since ${since}.`,
            missing: (source: string) => `The catalog is not complete: ${source} did not answer.`,
            commitMismatch: (build: string) =>
                `The docs page and the changelogs name different commits for build ${build}.`,
            docsPage: 'the docs page',
            changelog: 'the changelogs',
            branch: (name: string) => `branch ${name}`,
            longPath: (length: string) =>
                `The data folder is ${length} characters long: FXServer fails on paths past 260.`,
            lowDisk: (free: string) => `Only ${free} are free on the disk of the data folder.`,
            hide: 'Hide',
            hideLabel: 'Hide this notice',
        },
        build: {
            title: 'FXServer build',
            running: 'Running',
            stopped: 'Stopped',
            nextStart: 'Next start',
            none: 'No build is selected yet.',
            pending: 'Pending',
            atRestart: (when: string) => `at the next scheduled restart (${when})`,
            atRestartUnknown: 'at the next scheduled restart',
            lastHealthy: 'Last healthy',
            since: (when: string) => `since ${when}`,
            probation: 'Probation',
            passesAt: (when: string) => `passes at ${when}`,
            waiting: 'waiting for FXServer to be ready',
            restartNeeded: (running: string, next: string) =>
                `Restart needed: FXServer runs ${running}, ${next} starts next.`,
            cancelPending: 'Cancel',
            pendingCancelled: 'The pending build is cancelled.',
        },
        job: {
            title: 'Job',
            install: (build: string) => `Install of build ${build}`,
            import: (build: string) => `Import of build ${build}`,
            steps: {
                preCheck: 'Checking',
                download: 'Downloading',
                verify: 'Verifying',
                extract: 'Extracting',
                copy: 'Copying',
                validate: 'Validating',
                commit: 'Installing',
            } satisfies Record<JobStep, string>,
            during: {
                preCheck: 'checking',
                download: 'downloading',
                verify: 'verifying',
                extract: 'extracting',
                copy: 'copying',
                validate: 'validating',
                commit: 'installing',
            } satisfies Record<JobStep, string>,
            progress: 'Progress',
            downloaded: (received: string, total: string) => `${received} of ${total}`,
            rate: (rate: string) => `${rate}/s`,
            left: (left: string) => `${left} left`,
            files: (done: number, total: number | null) =>
                total === null
                    ? count(done, { one: 'file', other: 'files' })
                    : `${formatNumber('en', done)} of ${count(total, { one: 'file', other: 'files' })}`,
            cancel: 'Cancel',
            cancelling: 'Cancelling',
            committing: 'The build is being installed: it can no longer be cancelled.',
            installed: 'Installed.',
            use: 'Use this build…',
            failed: (step: string, reason: string) => `Failed while ${step}: ${reason}`,
            retry: 'Try again',
            cancelled: 'Cancelled. The partial files were removed.',
            longPath: 'The data folder is long: FXServer may fail on the deepest paths of this build.',
            freeSpaceUnknown: 'The free space of the disk could not be read.',
            failures: {
                unreachable: 'Cfx.re could not be reached.',
                httpStatus: 'Cfx.re answered with an error.',
                archiveTooLarge: 'the archive is larger than Observer accepts.',
                badLength: 'the archive did not have the length Cfx.re announced.',
                truncated: 'the download was cut short.',
                archiveChanged:
                    'the archive differs from the one first installed for this build, so it is not trusted.',
                diskSpace: 'the disk is too full.',
                unsafeEntry: 'the archive holds an entry Observer refuses.',
                damaged: 'the archive is damaged.',
                layout: 'the build is incomplete.',
                signature: 'FXServer.exe is not signed by Rockstar Games.',
                alreadyInstalled: 'the build is installed already.',
                cancelled: 'it was cancelled.',
                interrupted: 'Observer stopped.',
                io: "a file could not be read or written. Observer's log says which.",
                notAFolder: 'the folder does not exist, or is not a folder.',
                link: 'the folder is, or holds, a link Observer does not copy.',
                insideArtifacts: "the folder is inside Observer's artifacts folder.",
                tooManyFiles: 'the folder holds more than 20,000 entries.',
                tooLarge: 'the folder is larger than 2 GB.',
            } satisfies Record<InstallFailure['code'], string>,
            httpStatus: (status: number) => `Cfx.re answered HTTP ${status}.`,
            layout: {
                missing: (path: string) => `${path} is missing.`,
                notPlain: (path: string) => `${path} is a link, or not the kind of entry it should be.`,
                notExecutable: (path: string) => `${path} is not executable.`,
            },
            unsafeEntry: (entry: string) => `the archive holds an entry that ${entry}`,
            refusals: {
                empty: 'has an empty name.',
                tooLong: 'has a name too long.',
                control: 'has a control character in its name.',
                backslash: 'has a backslash in its name.',
                colon: 'has a colon in its name.',
                trailingDot: 'ends with a dot or a space, which Windows drops.',
                device: 'names a Windows device.',
                absolute: 'is an absolute path.',
                parent: 'climbs out of its folder with ...',
                duplicate: 'appears twice.',
                insideFile: 'lies inside a path another entry writes as a file.',
                insideLink: 'lies inside a symbolic link.',
                zipLink: 'is a symbolic link, which a Windows build never holds.',
                encrypted: 'is encrypted.',
                method: 'is compressed in a way Observer does not read.',
                entryType: 'is a device, a pipe or a socket.',
                linkOutsideAlpine: 'links to an absolute path outside alpine.',
                linkEscapes: 'links outside the build.',
                linkLoop: 'is part of a loop of symbolic links.',
                tooManyEntries: 'is past the 20,000 entries a build may have.',
                tooLarge: 'takes the build past 2 GB unpacked.',
            },
            diskSpace: (needed: string, available: string) =>
                `the disk has ${available} free, ${needed} are needed.`,
        },
        installed: {
            title: 'Installed builds',
            empty: 'No build is installed yet. Install one below, or import a folder of yours.',
            build: 'Build',
            state: 'State',
            installedAt: 'Installed',
            by: 'By',
            size: 'Size',
            actions: 'Actions',
            imported: 'Imported',
            commandLine: 'command line',
            copyCommit: (commit: string) => `Copy commit ${commit}`,
            copyBlocked: (commit: string) => `Copying is blocked here. The commit is ${commit}.`,
            chips: {
                running: 'Running',
                nextStart: 'Next start',
                pending: 'Pending',
                lastHealthy: 'Last healthy',
                probation: 'On probation',
                recommended: 'Recommended',
                latest: 'Latest',
                broken: 'Broken',
                endOfSupport: (day: string) => `End of support ${day}`,
                endOfLife: 'End of life',
            },
            use: 'Use…',
            delete: 'Delete…',
            protectedBy: {
                selected: 'It is the build FXServer starts next.',
                pending: 'It waits for the next scheduled restart.',
                running: 'It is the build FXServer runs.',
                lastHealthy: 'It is the last healthy build, which a failed switch goes back to.',
                probation: 'It is on probation.',
            } satisfies Record<Protection, string>,
            current: 'It is the build FXServer starts next already.',
            brokenReason: 'It is broken: delete it, then install it again.',
            archiveChanged:
                'It came from an archive other than the one first installed for it, so it is not trusted.',
            keep: (builds: number) =>
                `Keep ${count(builds, { one: 'build', other: 'builds' })} besides the ones in use`,
            cleanup: 'Clean up…',
            importBuild: 'Import a build…',
            notPermitted: 'Your account cannot change builds.',
            ownerOnly: 'Only the owner imports a folder of the host.',
            deleted: (build: string) => `Build ${build} is deleted.`,
        },
        available: {
            title: 'Available builds',
            filters: 'Filters',
            recommended: 'Recommended',
            latest: 'Latest',
            supportedOnly: 'Supported only',
            showEndOfLife: 'Show end of life',
            notListed: 'Not listed any more',
            search: 'Search by number or commit',
            build: 'Build',
            channel: 'Channel',
            published: 'Published',
            support: 'Support',
            size: 'Size',
            action: 'Action',
            install: 'Install…',
            isInstalled: 'Installed',
            noArchive: (platform: string) => `No archive for ${platform}`,
            noCommit: 'Its commit is unknown',
            unknownDate: 'unknown',
            never: 'never',
            refresh: 'Refresh',
            loading: 'Reading the catalog',
            failed: 'The catalog could not be read.',
            none: 'No build matches these filters.',
            listed: (time: string) => `Listed at ${time}`,
            supports: {
                supported: 'Supported',
                endOfSupport: 'End of support',
                endOfLife: 'End of life',
                unknown: 'Unknown',
            } satisfies Record<SupportLevel, string>,
            supportEnds: (day: string) => `support ends ${day}`,
            lifeEnds: (day: string) => `life ends ${day}`,
            platforms: { windows: 'Windows', linux: 'Linux' } satisfies Record<BuildPlatform, string>,
            started: (build: string) => `The install of build ${build} started.`,
        },
        settings: {
            title: 'Settings',
            readOnly: 'Your account cannot change these settings.',
            keepBuilds: 'Builds to keep',
            keepBuildsHint: 'Besides the ones in use: 1 to 20.',
            autoRollback: 'Go back to the last healthy build when a new one fails',
            autoRollbackHint:
                'A build that crashes or stops answering before it stayed up 10 minutes is replaced by the last healthy one, which FXServer restarts on.',
            branches: 'Test branches',
            branchesHint: 'Builds of these runtime.fivem.net branches are listed too: 10 at most.',
            branchName: 'Branch name',
            addBranch: 'Add a branch',
            removeBranch: (name: string) => `Remove ${name}`,
            noBranch: 'No test branch.',
            problems: {
                outOfRange: 'Keep 1 to 20 builds.',
                tooMany: '10 branches at most.',
                invalid: 'Use up to 4 parts joined by /, each of lower-case letters, digits, - and _.',
                master: 'master is always listed.',
                duplicate: 'This branch is listed already.',
            } satisfies Record<ArtifactSettingsProblem, string>,
            changes: (changes: number) => `${count(changes, { one: 'change', other: 'changes' })} not saved`,
            discard: 'Discard',
            save: 'Save',
            saved: 'Settings saved.',
            discarded: 'Changes discarded.',
            changedElsewhere:
                'These settings were changed in another session. Discard your changes to see theirs, then make yours again.',
        },
        install: {
            title: (build: string) => `Install build ${build}?`,
            channel: 'Channel',
            published: 'Published',
            support: 'Support',
            size: 'Download',
            free: 'Free on the disk',
            endOfLife:
                'Install it anyway: players cannot join a server running a build past its end of life.',
            confirm: 'Install',
        },
        use: {
            title: (build: string) => `Use build ${build}?`,
            when: 'When',
            now: 'Restart now',
            nowPlayers: (players: number) =>
                players === 0
                    ? 'Nobody is connected.'
                    : `${count(players, { one: 'player is', other: 'players are' })} connected. They will be disconnected.`,
            nextStart: 'At the next start',
            nextStartRunning: (running: string) =>
                `FXServer keeps running ${running} until it starts again, a restart after a crash included.`,
            nextStartStopped: 'FXServer starts this build the next time it starts.',
            scheduled: 'At the next scheduled restart',
            noRestart: 'No scheduled restart is ahead.',
            notRunning: 'FXServer is not running.',
            stopping: 'FXServer is stopping.',
            notPermitted: 'Your account cannot restart FXServer.',
            replaces: (build: string) => `Replaces the pending build ${build}.`,
            kvp: "FXServer may delete the server's KVP data going back this far. Type 6449 to confirm.",
            kvpLabel: 'Type 6449',
            endOfLife: 'Use it anyway: players cannot join a server running a build past its end of life.',
            confirm: 'Use build',
            kept: (build: string) => `FXServer runs build ${build} already: it starts next again.`,
            done: {
                now: (build: string) => `FXServer is restarting on build ${build}.`,
                nextStart: (build: string) => `Build ${build} starts at the next start.`,
                scheduledRestart: (build: string) =>
                    `Build ${build} is applied at the next scheduled restart.`,
            } satisfies Record<ApplyMode, (build: string) => string>,
        },
        delete: {
            title: (build: string) => `Delete build ${build}?`,
            frees: (size: string) => `It frees ${size}.`,
            gone: 'Cfx.re no longer lists it: it cannot be installed again.',
            confirm: 'Delete',
        },
        cleanup: {
            title: 'Clean up',
            loading: 'Working out what goes',
            none: 'Nothing to remove: every build is in use or among the ones kept.',
            gone: 'cannot be installed again',
            freed: (size: string) => `${size} freed`,
            confirm: (builds: number) => `Delete ${count(builds, { one: 'build', other: 'builds' })}`,
            done: (builds: number, size: string) =>
                `${count(builds, { one: 'build', other: 'builds' })} deleted, ${size} freed.`,
            skipped: (builds: string) => `Kept because they are in use: ${builds}.`,
        },
        import: {
            title: 'Import a build',
            folder: 'Build folder',
            folderHint: 'The folder of FXServer.exe on Windows, or the one holding alpine on Linux.',
            build: 'Build number',
            buildHint: 'The build this folder holds, 4 to 7 digits: a folder does not say it reliably.',
            about: "Observer copies the folder into its data folder and never changes the original. Crash dumps and Observer's bridge are left out.",
            invalidBuild: 'Use 4 to 7 digits.',
            confirm: 'Import',
            started: (build: string) => `The import of build ${build} started.`,
        },
        failed: (action: string, reason: string) => `${action} failed: ${reason}`,
        cancel: 'Cancel',
    },
    tiles: {
        unknown: 'n/a',
        players: {
            label: 'Players',
            ping: 'Ping',
            of: (slots: string) => `/ ${slots}`,
            pings: (average: string, p95: string) => `${average} · p95 ${p95}`,
            lastHour: 'Last hour',
            joinedLeft: (joined: string, left: string) => `${joined} joined · ${left} left`,
            trend: 'Players over the last hour',
        },
        tick: {
            label: 'svMain tick',
            unit: 'ms avg',
            overBudget: 'Over the 50 ms budget',
            others: 'svNetwork · svSync',
            lastHitch: 'Last hitch',
            hitchAt: (ms: string, time: string) => `${ms} at ${time}`,
            noHitch: 'None in the last hour',
            trend: 'svMain tick time over the last hour',
        },
        cpu: {
            label: 'FXServer CPU',
            unit: 'of one core',
            machine: 'Of the machine',
            main: 'svMain',
            networkSync: 'svNetwork · svSync',
            runtimes: 'JS · Mono · other',
            user: 'user',
            kernel: 'kernel',
            threadsHandles: 'Threads · handles',
            children: 'Child processes',
            childrenValue: (processes: number, cpu: string) =>
                `${count(processes, { one: 'process', other: 'processes' })} · ${cpu}`,
            trend: 'FXServer CPU over the last hour',
        },
        memory: {
            label: 'FXServer memory',
            unit: (unit: string) => `${unit} private`,
            possibleLeak: 'Possible leak',
            sinceStart: 'Since start',
            perHour: (growth: string) => `${growth}/h`,
            workingSet: 'Working set',
            withPeak: (now: string, peak: string) => `${now} · peak ${peak}`,
            heap: 'JS heap',
            io: 'I/O read · write',
            handles: 'Handles',
            handlesSinceStart: (growth: string) => `${growth}/h since start`,
            perSecond: (bytes: string) => `${bytes}/s`,
            trend: 'FXServer private memory over the last hour',
        },
        host: {
            label: 'Host',
            unit: 'CPU',
            memory: 'RAM',
            disk: 'Disk free · latency',
            idle: 'idle',
            network: 'Network in · out',
            clock: 'CPU clock',
            // Windows gives the rated speed, which a boosting processor goes past.
            clockValue: (now: string, rated: string) => `${now} GHz · ${rated} GHz rated`,
            limited: (share: string) => ` · limited to ${share}`,
            steal: 'Steal',
            pressure: 'Pressure',
            pressureValue: (cpu: string, memory: string, io: string) =>
                `cpu ${cpu} · memory ${memory} · io ${io}`,
            trend: 'Host CPU over the last hour',
        },
        observer: {
            label: 'Observer',
            running: 'Running',
            unreachable: 'Cannot be reached',
            unit: (unit: string) => `${unit} RAM`,
            cpu: 'CPU',
            clients: 'Dashboard clients',
            databases: 'Databases',
            databasesValue: (state: string, metrics: string) => `${state} · ${metrics}`,
            version: 'Version',
            versionValue: (version: string, uptime: string) => `${version} · up ${uptime}`,
            trend: 'Observer memory over the last hour',
        },
        ratio: (part: string, whole: string) => `${part} / ${whole}`,
        pair: (first: string, second: string) => `${first} · ${second}`,
        triple: (first: string, second: string, third: string) => `${first} · ${second} · ${third}`,
    },
    distribution: {
        key: (part: string, share: string) => `${part} ${share}`,
    },
    tree: {
        title: 'FXServer tree',
        now: 'Now',
        children: 'Child processes',
        startedLastHour: 'Started in the last hour',
        wholeTree: 'Whole tree CPU',
        ofOneCore: (share: string) => `${share} of one core`,
        endedIncluded: 'ended processes included',
        memoryPeak: 'Tree memory peak',
        faults: 'Page faults',
        faultsValue: (perSecond: string, hard: string | null) =>
            hard === null ? `${perSecond}/s` : `${perSecond}/s · ${hard} hard/s`,
        threadsHandles: (threads: string, handles: string, growth: string | null) =>
            growth === null ? `${threads} · ${handles}` : `${threads} · ${handles} · ${growth}/h`,
    },
    threads: {
        title: 'Threads by CPU',
        meta: 'Last hour · % of one core',
        caption: "FXServer's threads by CPU over the last hour, in percent of one core",
        thread: 'Thread',
        share: 'Share of FXServer CPU',
        cpu: 'CPU',
        others: 'Others',
        unnamed: 'Unnamed',
        empty: 'No thread was measured in the last hour.',
        failed: 'The threads could not be read. They are read again in a moment.',
        loading: 'Loading the threads',
    },
    charts: {
        ranges,
        event: 'Event',
        down: 'Down',
        downFor: (duration: string, range: string) => `Down for ${duration} over ${range}.`,
        players: {
            title: 'Online players',
            range: 'Online players range',
            every: (step: string) => `Every ${step}`,
            players: 'Players',
            maxSlots: 'Max slots',
            minMax: 'Min–max',
            maxSlotsAt: (slots: string) => `Max slots ${slots}`,
            tooltip: (players: number) => count(players, { one: 'player', other: 'players' }),
            summary: (average: string, range: string, peak: string, at: string) =>
                `${average} on average over ${range}. Peak ${peak} at ${at}.`,
            summaryNow: (now: string, average: string, range: string, peak: string, at: string) =>
                `${now} online now, ${average} on average over ${range}. Peak ${peak} at ${at}.`,
            empty: 'Nothing was measured over this range.',
            failed: 'The players chart could not be read. It is read again in a moment.',
            unavailable: 'The players chart could not be shown. Reload the page to see it.',
            loading: 'Loading the players chart',
        },
        tick: {
            title: 'Tick time',
            range: 'Tick time range',
            meta: 'Milliseconds, log scale',
            main: 'svMain',
            network: 'svNetwork',
            sync: 'svSync',
            budget: (ms: string) => `${ms} ms budget`,
            tooltip: (main: string, network: string, sync: string) =>
                `svMain ${main} · svNetwork ${network} · svSync ${sync}`,
            average: 'svMain average',
            ms: 'ms',
            mainOver: (ms: string) => `svMain over ${ms} ms`,
            ofTicks: 'of ticks',
            othersOver: (network: string, sync: string) => `svNetwork · svSync over ${network} · ${sync} ms`,
            longest: 'Longest hitch',
            msAt: (time: string) => `ms at ${time}`,
            none: 'None',
            empty: 'Nothing was measured over this range.',
            failed: 'The tick time chart could not be read. It is read again in a moment.',
            unavailable: 'The tick time chart could not be shown. Reload the page to see it.',
            loading: 'Loading the tick time chart',
        },
        markers: {
            start: 'Start',
            restart: 'Restart',
            crash: 'Crash',
            build: (build: number | null) => (build === null ? 'Build changed' : `Build ${build}`),
            backOnline: (time: string) => ` · back online ${time}`,
            inWords: (marker: string, time: string, backOnline: string | null) =>
                backOnline === null ? `${marker} ${time}` : `${marker} ${time}, back online ${backOnline}`,
            listed: (markers: readonly string[], more: number) =>
                more > 0 ? `${markers.join(' · ')} and ${more} more` : markers.join(' · '),
        },
        attribution: 'Charts by',
        library: 'TradingView Lightweight Charts™',
    },
    schedule: {
        title: 'Schedule',
        gloss: 'When FXServer restarts by itself.',
        zone: (zone: string, offset: string) => `Times in ${zone} (${offset})`,
        zoneUnknown: (zone: string) => `Times in UTC: this browser does not know ${zone}`,
        loading: 'Reading the schedule',
        unknown: 'The schedule is not known while the live link is down.',
        outOfDate: 'The live link is down: the schedule shown may be out of date. Nothing typed is lost.',
        nextTitle: 'Next restart',
        when: (day: string, left: string) => `${day} · in ${left}`,
        dayTime: (day: string, time: string) => `${day} ${time}`,
        issueOf: (subject: string, problem: string) => `${subject}: ${problem}`,
        fromSchedule: 'From the schedule.',
        oneOff: (by: string) => `One-off, planned by ${by}.`,
        delayedFrom: (time: string, by: string) => `Delayed from ${time} by ${by}.`,
        stopped: 'FXServer is stopped: the schedule only restarts a running server.',
        countsFromStart: 'Counts from the next start of FXServer.',
        emptyTitle: 'No scheduled restart',
        emptyBody:
            'Add a fixed time or an interval below. Your resources hear of each restart before it comes.',
        actions: 'Restart actions',
        skip: 'Skip',
        restore: 'Restore',
        delay: 'Delay',
        delayBy: (amount: string) => `Delay by ${amount}`,
        restartOnce: 'Restart once…',
        cancelOneOff: 'Cancel the one-off',
        notPermittedPlan: 'Your account cannot change the schedule.',
        notPermittedOnce: 'Your account cannot restart FXServer.',
        notRunning: 'FXServer is not running.',
        stoppingOnce: 'FXServer is stopping: plan the restart once it runs again.',
        hungOnce: 'FXServer is not responding and is being stopped: plan the restart once it runs again.',
        oneSkip: (time: string) => `Restore ${time} first: one restart can be skipped at a time.`,
        unavailable: (action: string, reason: string) => `${action} does nothing now. ${reason}`,
        failed: (action: string, error: string) => `${action} did not go through. ${error}`,
        dismiss: 'Dismiss',
        skipped: (time: string) => `The restart at ${time} is skipped.`,
        restored: (time: string) => `The restart at ${time} is back.`,
        delayed: (time: string) => `The restart moved to ${time}.`,
        planned: (when: string) => `Planned for ${when}.`,
        cancelled: 'The single restart is cancelled.',
        onceTitle: 'Restart FXServer once',
        onceWhen: 'When',
        onceIn: 'In',
        onceAt: 'At',
        onceMinutes: 'Minutes from now',
        onceTime: 'Time',
        onceMinutesShort: (minutes: number) => `${formatNumber('en', minutes)} min`,
        onceHint:
            'It replaces any single restart planned before. Resources hear of it at once when it is inside the warnings, then at each one.',
        onceSubmit: 'Plan the restart',
        onceRange: `A whole number of minutes, from 1 to ${formatNumber('en', 1440)}.`,
        onceTimeRequired: 'Pick a time, hours and minutes.',
        upcomingTitle: 'Upcoming',
        upcomingEmpty: 'No restart ahead.',
        skippedBy: (by: string) => `Skipped by ${by}`,
        skippedStart: (time: string) => `Skipped: FXServer started at ${time}`,
        delayedTo: (time: string) => `Delayed to ${time}`,
        moved: 'Moved by the clock change',
        settingsTitle: 'Settings',
        readOnly: 'Your account can see these settings, not change them.',
        enabled: 'Restart on a schedule',
        enabledHint: 'FXServer restarts at the times below, or at the interval, while it runs.',
        mode: 'Restarts',
        modes: {
            fixedTimes: 'Fixed times',
            interval: 'Interval',
        } satisfies Record<ScheduleMode, string>,
        times: 'Times',
        noTimes: 'No time yet.',
        removeTime: (time: string) => `Remove ${time}`,
        addTime: 'Add a time',
        timeIncomplete: 'Type a full time: hours and minutes.',
        listedAlready: 'Already in the list.',
        add: 'Add',
        every: 'Every',
        everyHours: 'Hours between two restarts',
        hoursFrom: 'hours from',
        from: 'Starting at',
        fill: 'Fill',
        fillLegend: 'Fill the list with times a few hours apart',
        fillHint: 'Fills the list with times that many hours apart. Only the list is kept.',
        intervalHours: 'Hours',
        intervalMinutes: 'Minutes',
        intervalHint: 'Counts from each start of FXServer, a restart after a crash included.',
        timeZone: 'Time zone',
        hostZone: (zone: string | null) => (zone === null ? "Host's zone" : `Host's zone (${zone})`),
        timeZoneHint:
            'Observer has the last word: a zone its own time zone database does not know is refused.',
        warnings: 'Warnings',
        warningMinutes: (minutes: number) => `${formatNumber('en', minutes)} min`,
        removeWarning: (minutes: number) =>
            `Remove the warning ${count(minutes, { one: 'minute', other: 'minutes' })} before`,
        addWarning: 'Add a warning, in minutes before',
        resetWarnings: 'Reset to the defaults',
        warningsHint:
            'Observer raises observer:restartScheduled at each. Your resources decide what players see.',
        skipAfterStart: 'Skip a fixed time this soon after a start, in minutes',
        skipAfterStartHint:
            'After a start the schedule did not make, by hand or after a crash, a fixed time that comes this soon is skipped. 0 turns this off.',
        changes: (value: number) => `${count(value, { one: 'unsaved change', other: 'unsaved changes' })}`,
        discard: 'Discard',
        save: 'Save the schedule',
        saved: 'The schedule is saved. It applies at once.',
        discarded: 'Changes discarded.',
        changedElsewhere:
            'Someone saved the schedule since you opened it. Discard your changes to see theirs, then make yours again.',
        problems: {
            required: 'Add at least one time, or turn the schedule off.',
            tooMany: 'Too many: remove some.',
            invalidTime: 'Use a time between 00:00 and 23:59.',
            duplicate: 'This one is listed twice.',
            tooClose: 'Keep times at least 30 minutes apart, across midnight too.',
            outOfRange: 'This number is out of range.',
            unknownTimeZone: 'Observer does not know this time zone.',
        } satisfies Record<ScheduleProblem, string>,
        ranges: {
            intervalMinutes: 'Whole minutes, from 30 minutes to 7 days.',
            warning: 'A whole number of minutes, from 1 to 240.',
            skipAfterStart: 'A whole number of minutes, from 0 to 720.',
            times: 'At most 48 times.',
            warnings: 'At most 16 warnings.',
        },
    },
    alerts: {
        title: 'Alerts',
        meta: 'Last 24 hours',
        caption: 'Alerts of the last 24 hours, newest first',
        securityAlert: 'Security alert',
        crash: 'Crash',
        hang: 'Hang',
        shellStarted: "FXServer's tree started",
        crashedIn: 'in',
        crashed: 'FXServer crashed',
        exitedWith: (exit: string) => `FXServer exited with ${exit}`,
        stuckIn: (thread: string) => `${thread} was stuck in`,
        stuck: (thread: string) => `${thread} stopped responding`,
        busy: (thread: string) => `${thread} was busy on the CPU`,
        waitingIn: (thread: string) => `${thread} was waiting in`,
        times: (value: number) => `${formatNumber('en', value)} times`,
        parent: (name: string) => `parent ${name}`,
        leftRunning: 'left running, audited',
        inDays: (value: number) => `${ordinal(value)} time in 7 days`,
        exitCode: (code: string) => `exit code ${code}`,
        signal: (value: number) => `signal ${value}`,
        signals: {
            1: 'SIGHUP',
            2: 'SIGINT',
            4: 'SIGILL',
            6: 'SIGABRT',
            7: 'SIGBUS',
            8: 'SIGFPE',
            9: 'SIGKILL',
            11: 'SIGSEGV',
            13: 'SIGPIPE',
            15: 'SIGTERM',
        } as Record<number, string>,
        thread: (name: string) => `thread ${name}`,
        snapshot: 'dump taken before the kill',
        open: 'Open Diagnostics',
        needsPermission: 'Needs the View diagnostics permission',
    },
    diagnostics: {
        title: 'Diagnostics',
        gloss: 'What FXServer starts, where it connects, and why it stopped.',
        forbidden: 'Your account cannot see the diagnostics.',
        loading: 'Reading the diagnostics',
        unavailable: 'The diagnostics are not known while the live link is down.',
        crashes: {
            title: 'Crashes and hangs',
            meta: (files: string, maxFiles: string, bytes: string, maxBytes: string) =>
                `Dumps ${files} of ${maxFiles} · ${bytes} of ${maxBytes}`,
            free: (bytes: string) => `${bytes} free`,
            caption: 'Crashes, hangs and dumps found, newest first',
            when: 'When',
            what: 'What',
            group: 'Group',
            files: 'Files',
            actions: 'Actions',
            kinds: { crash: 'Crash', hang: 'Hang', found: 'Found' },
            found: 'Dumps no crash or hang explains',
            hung: 'FXServer stopped responding',
            exited: (exit: string) => `FXServer exited with ${exit}`,
            reading: 'Reading the dump…',
            unreadable: 'The dump could not be read',
            noDump: 'No dump to read',
            thread: (name: string) => `thread ${name}`,
            build: (build: number) => `build ${build}`,
            inDays: (value: number) => `${formatNumber('en', value)} in 7 days`,
            fileKinds: {
                minidump: 'minidump',
                fullDump: 'full dump',
                liveDump: 'live dump',
                hangDump: 'hang dump',
                hangReport: 'thread report',
                other: 'file',
            },
            staysOnHost: 'stays on the host',
            removed: 'removed',
            details: 'Details',
            hideDetails: 'Hide details',
            frames: 'Stack scan, approximate without symbols',
            noFrames: 'No frame was read.',
            watchdog: 'Watchdog stack',
            threads: 'Threads',
            threadsCaption: 'svMain first, then the busiest threads',
            threadName: 'Thread',
            threadState: 'State',
            threadCpu: 'CPU',
            threadCall: 'System call',
            syscallsHidden: 'This host hides the system calls threads wait in (kernel.yama.ptrace_scope).',
            download: (name: string) => `Download ${name}`,
            deleteFiles: 'Delete files…',
            whatChanged: 'What changed before this run',
            deleteTitle: 'Delete these files?',
            deleteBody:
                "Its dumps hold FXServer's memory. They are deleted from the host; the analysis stays.",
            deleteConfirm: 'Delete files',
            deleted: 'Files deleted from the host',
            emptyTitle: 'No crash or hang recorded',
            emptyBody: 'When FXServer crashes or stops responding, what Observer reads of it shows here.',
        },
        processes: {
            title: 'Processes FXServer started',
            meta: 'Watched as they start',
            caption: 'Processes FXServer started, latest first',
            started: 'Started',
            process: 'Process',
            kind: 'Kind',
            parent: 'Parent',
            run: 'Run',
            count: 'Count',
            state: 'State',
            kinds: {
                shell: 'Security alert',
                other: 'Unexpected',
                dumpServer: 'Crash dump server',
                consoleHost: 'Console host',
                node: 'Node.js of a resource',
            },
            unread: 'ended before it was read',
            cut: 'cut at 4 KB',
            more: 'Show the whole command line',
            less: 'Show less',
            generation: (value: number) => `#${value}`,
            times: (value: number) => `×${formatNumber('en', value)}`,
            running: 'Running',
            ended: (time: string) => `Ended ${time}`,
            limits: "A program started through a service, the task scheduler or WMI is not FXServer's child, and shows as the tool that launched it. On Linux a program that ends within a quarter of a second can be missed.",
            emptyTitle: 'Nothing started yet',
            emptyBody: "FXServer's crash dump server and console host show here once it runs.",
        },
        connections: {
            title: 'Connections',
            meta: 'Every 2 s · last 24 hours',
            destinations: 'Destinations',
            destinationsCaption: 'Where the processes of FXServer connected in the last 24 hours',
            listening: 'Listening',
            listeningCaption: 'Where the processes of FXServer listen',
            destination: 'Destination',
            scope: 'Scope',
            protocol: 'Protocol',
            process: 'Process',
            openNow: 'Open now',
            peak: 'Peak',
            lastSeen: 'Last seen',
            endpoint: 'Endpoint',
            role: 'Role',
            inbound: 'Inbound now',
            new: 'New',
            game: 'Game endpoint',
            scopes: { loopback: 'Loopback', private: 'Private network', public: 'Public' },
            services: {
                mysql: 'MySQL',
                postgresql: 'PostgreSQL',
                redis: 'Redis',
                mongodb: 'MongoDB',
                https: 'HTTPS',
                http: 'HTTP',
                dns: 'DNS',
            },
            protocols: { tcp: 'TCP', udp: 'UDP' },
            none: 'No connection in the last 24 hours.',
            noListener: 'Nothing listens while FXServer is stopped.',
            footer: 'Connections shorter than 2 s may not show. Windows lists UDP sockets without their remote end.',
        },
        exceptions: {
            '0xC0000005': 'Access violation',
            '0xC000001D': 'Illegal instruction',
            '0xC0000017': 'Out of memory',
            '0xC0000094': 'Integer division by zero',
            '0xC00000FD': 'Stack overflow',
            '0xC0000374': 'Heap corruption',
            '0xC0000409': 'Stack buffer overrun',
            '0x80000003': 'Breakpoint',
        } as Record<string, string>,
        threadStates: {
            running: 'running',
            sleeping: 'sleeping',
            diskWait: 'waiting on a disk',
            stopped: 'stopped',
            zombie: 'ended',
            other: 'other',
        },
        syscallGroups: {
            lock: 'waiting for a lock',
            io: 'waiting on input or output',
            wait: 'waiting for events',
            sleep: 'sleeping',
        },
    },
    players: {
        title: 'Players',
        gloss: 'Who plays, for how long, and how often.',
        zone: (zone: string, offset: string) => `Days and times in ${zone} (${offset})`,
        zoneUnknown: (zone: string) => `Days in ${zone}, times in UTC: this browser does not know ${zone}`,
        timeConnected: 'Play time is time connected: loading screens and idle players count.',
        forgotten:
            'The player was forgotten: their profile, play time, names and the identifiers only they held are deleted.',
        online: 'Online',
        offline: 'Offline',
        copyLicence: (license: string) => `Copy the licence ${license}`,
        // A name the player chose that nothing was left of once cleaned of control characters.
        noName: 'No name',
        now: {
            title: 'Online now',
            meta: (players: number) => `${count(players, { one: 'player', other: 'players' })} · every 10 s`,
            caption: 'Players connected now, by server ID',
            player: 'Player',
            licence: 'Licence',
            session: 'This session',
            total: 'Total play time',
            serverId: (id: number) => `ID ${id}`,
            confirming: 'Confirming',
            confirmingHint: 'FXServer has not listed this player yet.',
            unconfirmed: 'Unconfirmed',
            unconfirmedHint: "FXServer's player list did not match: nothing is kept of who they are.",
            empty: 'Nobody is connected.',
            loading: 'Reading who is connected',
            failed: 'Who is connected could not be read. It is read again in 10 s.',
        },
        community: {
            title: 'Community',
            players: 'Unique players',
            newPlayers: 'New players',
            playTime: 'Play time',
            lastWeek: 'Last 7 days, today so far',
            before: 'The 7 days before',
            difference: 'Difference',
            period: 'Chart period',
            perDay: 'Per day',
            perWeek: 'Per week',
            tooltipPlayers: (players: number | null) =>
                players === null
                    ? 'Unique players not known'
                    : count(players, { one: 'unique player', other: 'unique players' }),
            tooltipOthers: (newPlayers: number, playTime: string) =>
                `${formatNumber('en', newPlayers)} new · ${playTime} played`,
            todaySoFar: 'today, so far',
            weekSoFar: 'this week, so far',
            weekOf: (first: string, last: string) => `${first} to ${last}`,
            summaryDays: (peak: number, day: string, players: number, newPlayers: number, playTime: string) =>
                `At most ${count(peak, { one: 'player', other: 'players' })} in a day, on ${day}. Today, so far: ${count(players, { one: 'player', other: 'players' })}, ${formatNumber('en', newPlayers)} new, ${playTime} played.`,
            summaryWeeks: (players: number, newPlayers: number, playTime: string) =>
                `This week, so far: ${count(players, { one: 'player', other: 'players' })}, ${formatNumber('en', newPlayers)} new, ${playTime} played. Unique players per week are known for the last two weeks only.`,
            empty: 'Nobody played in the last 30 days.',
            loading: 'Loading the community figures',
            failed: 'The community figures could not be read.',
        },
        list: {
            title: 'All players',
            search: 'Search players',
            searchHint: 'The start of a name, or a licence.',
            searchHintIdentifiers:
                'The start of a name, a licence, or any identifier with its prefix, such as discord:. Never an address.',
            submit: 'Search',
            clear: 'Clear',
            sort: 'Sort by',
            sorts: {
                playTime: 'Total play time',
                recent: 'Last 30 days',
                lastSeen: 'Last visit',
                firstSeen: 'First visit',
                sessions: 'Sessions',
            } satisfies Record<PlayerSort, string>,
            caption: (sort: string) => `Players, sorted by ${sort}, largest or latest first`,
            name: 'Name',
            licence: 'Licence',
            state: 'State',
            imported: (time: string) => `${time} before Observer`,
            empty: 'No player yet. A player shows here once FXServer has confirmed who they are.',
            noMatch: 'No player matches this search.',
            loadMore: 'Load more',
            shown: (players: number) => `${count(players, { one: 'player', other: 'players' })} shown`,
            allShown: (players: number) => `All ${count(players, { one: 'player', other: 'players' })} shown`,
            loading: 'Reading the players',
            failed: 'The players could not be read.',
        },
        profile: {
            back: 'All players',
            loading: 'Reading the player',
            failed: 'The player could not be read.',
            facts: 'About this player',
            licence: 'Licence',
            firstSeen: 'First visit',
            lastSeen: 'Last visit',
            imported: 'Imported from txAdmin',
            total: 'Total play time',
            seenByObserver: 'Seen by Observer',
            beforeObserver: 'Before Observer',
            inProgress: 'Session in progress',
            last7: 'Last 7 days',
            last30: 'Last 30 days',
            todaySoFar: 'Today, so far',
            sessions: 'Sessions',
            average: 'Average',
            longest: 'Longest',
            noSession: 'None seen by Observer yet',
            crashRate: 'Crashes',
            perHundredHours: 'per 100 h',
            serverRate: "The server's",
            rate: (rate: string) => `${rate} per 100 h`,
            crashesSeen: 'Crashes seen',
            unknown: 'n/a',
            activity: {
                title: 'Activity',
                meta: 'Play time per day, last 90 days',
                summary: (days: number, total: string) =>
                    days === 0
                        ? 'No play time in the last 90 days.'
                        : `Played on ${count(days, { one: 'day', other: 'days' })} of the last 90, ${total} in all.`,
                today: (time: string) => `Today, so far: ${time}.`,
                tooltip: (time: string) => `${time} played`,
                todaySoFar: 'today, so far',
                hours: (hours: string) => `${hours}h`,
            },
            names: {
                title: 'Names',
                caption: 'Names this player used, with when they were first and last seen',
                name: 'Name',
                firstSeen: 'First seen',
                lastSeen: 'Last seen',
            },
            drops: {
                title: 'Disconnections',
                crashes: 'Crashes',
                timeouts: 'Timeouts',
                kicks: 'Kicks',
                lifetime: 'Over every session Observer saw',
                caption: 'Their last disconnections, latest first',
                when: 'When',
                what: 'What',
                length: 'Session',
                by: 'by',
                empty: 'No disconnection is kept for this player.',
                categories: {
                    quit: 'Quit',
                    kick: 'Kicked',
                    timeout: 'Timed out',
                    crash: 'Crashed',
                    security: 'Security',
                    serverStop: 'Server stopped',
                    serverCrash: 'Server crashed',
                    unknown: 'Unknown',
                } satisfies Record<DropCategory, string>,
                kinds: {
                    exit: 'left the game',
                    disconnect: 'disconnected',
                    otherServer: 'joined another server',
                    reconnect: 'reconnected',
                    editor: 'opened the Rockstar Editor',
                    reload: 'reloaded the game',
                    quitMessage: 'quit with a message',
                    resource: 'by a resource',
                    connection: 'connection lost',
                    pendingCommands: 'too many commands waiting',
                    missedFrames: 'missed frames',
                    replaced: 'replaced by a new connection',
                    stateBagRate: 'too many state bag changes',
                    netEventRate: 'too many net events',
                    latentNetEventRate: 'too many latent net events',
                    commandRate: 'too many commands',
                    netGameEventRate: 'too many game events',
                    serverInitiated: 'refused by the server',
                    stop: 'stopped',
                    restart: 'restarted',
                    scheduled: 'scheduled restart',
                    shutdown: 'Observer stopped',
                    crashLoop: 'crash loop',
                    console: 'quit in the console',
                    crash: 'crash',
                    hang: 'stopped responding',
                    lost: 'lost when Observer stopped',
                    missed: 'missed',
                    other: 'other',
                } satisfies Record<DropKind, string>,
            },
            identifiers: {
                title: 'Identifiers',
                caption: 'Identifiers this player showed besides the licence',
                kind: 'Kind',
                value: 'Value',
                firstSeen: 'First seen',
                lastSeen: 'Last seen',
                seen: 'Seen',
                times: (times: number) => count(times, { one: 'time', other: 'times' }),
                kinds: {
                    license2: 'Licence 2',
                    steam: 'Steam',
                    discord: 'Discord',
                    fivem: 'Cfx.re',
                    xbl: 'Xbox Live',
                    live: 'Microsoft',
                    token: 'Hardware token',
                    ip: 'Address',
                } satisfies Record<IdentifierKind, string>,
                copy: (kind: string, value: string) => `Copy the ${kind} identifier ${value}`,
                sameAddress: (others: number) =>
                    others === 0
                        ? 'An address no other licence used'
                        : `Same address as ${count(others, { one: 'other licence', other: 'other licences' })}`,
                sharedWith: (others: number) =>
                    `Shared with ${count(others, { one: 'other licence', other: 'other licences' })}`,
                imported: 'Imported',
                importedHint: 'From txAdmin, not seen by Observer since.',
                empty: 'No identifier besides the licence.',
            },
            linked: {
                title: 'Linked licences',
                caption: 'Other licences that share an identifier with this one, directly or through another',
                player: 'Player',
                through: 'Linked through',
                link: 'Link',
                strong: 'Strong',
                weak: 'Weak',
                strength: (strength: string) => `· ${strength}`,
                direct: 'Shares an identifier',
                indirect: 'Through another licence',
                empty: 'No other licence shares an identifier with this one.',
                note: 'A strong link goes through an identifier Observer saw itself, shown by 3 licences at most. A weak one goes through an address, an identifier many licences show, or one only imported from txAdmin.',
            },
            forget: {
                button: 'Forget this player…',
                title: (name: string) =>
                    name === '' ? 'Forget this player, who has no name?' : `Forget ${name}?`,
                lead: 'For an erasure request. Observer deletes, now and for good:',
                goes: [
                    'their profile and play time, day by day',
                    'the names they used',
                    'the identifiers no other licence holds; those another licence shares stay with it',
                    'the texts of their sessions: crash and kick reasons',
                ],
                stays: 'The statistics keep their sessions as counts, with nothing that names them.',
                backups:
                    'A licence forgotten stays forgotten: restoring a daily, pre-migration or manual backup made before now does not bring it back, and neither does an import from txAdmin. If they join again, they start a new profile.',
                confirm: (code: string) => `Type ${code} to confirm`,
                submit: 'Forget the player',
            },
        },
        data: {
            title: 'Player data',
            meta: 'Owner only',
            retention: 'Retention',
            retentionLead:
                'How long Observer keeps what it knows of players. Each identifier, name and day expires on its own, after it was last seen.',
            linkDays: 'Identifiers and names',
            addressDays: 'Address fingerprints',
            dayDays: 'Play time per day',
            profileDays: 'Profiles',
            ranges: {
                linkDays: "Days after each was last seen, from 30 up to the profiles' retention.",
                addressDays: "Days, from 7 up to the identifiers' retention.",
                dayDays: "Days, from 30 up to the profiles' retention.",
                profileDays:
                    'Days after the last visit, 365 or more. Everything that hangs on a profile goes with it.',
            },
            outOfRange: (range: string) => `Out of range. ${range}`,
            keepForGood: 'Keep profiles for good',
            keepForGoodHint: 'A profile and its play time stay until the player is forgotten on request.',
            changes: (changes: number) => `${count(changes, { one: 'change', other: 'changes' })} not saved`,
            discard: 'Discard',
            discarded: 'Changes discarded.',
            save: 'Save the retention',
            saved: 'Retention saved. What it no longer keeps goes at the next daily upkeep.',
            loading: 'Reading the retention',
            failed: 'The retention could not be read.',
            import: 'Import from txAdmin',
            importLead:
                "Takes in the play time, first and last visits, names and identifiers of txAdmin's players database. Observer only reads the file; notes, bans, warnings and the whitelist stay out. Importing it again changes nothing, and forgotten players stay forgotten.",
            path: 'Players database',
            pathHint: 'The full path on this host of <txData>/<profile>/data/playersDB.json.',
            start: 'Import',
            started: 'The file was read and checked whole. Its players are being written.',
            running: 'An import is running already: wait for it to end.',
            progress: 'Import progress',
            writing: (written: number, read: number) =>
                `Writing ${formatNumber('en', written)} of ${count(read, { one: 'player', other: 'players' })}`,
            finished: (at: string, imported: number, updated: number, forgotten: number, invalid: number) =>
                `Imported ${at}: ${count(imported, { one: 'new profile', other: 'new profiles' })}, ${formatNumber('en', updated)} updated, ${formatNumber('en', forgotten)} skipped as forgotten, ${formatNumber('en', invalid)} without a valid licence.`,
            stopped: (at: string, written: number, read: number) =>
                `The import of ${at} stopped after ${formatNumber('en', written)} of ${count(read, { one: 'player', other: 'players' })}. Import the file again to finish: what was written stays.`,
        },
    },
    drops: {
        title: 'Player drops',
        gloss: 'How sessions end, and whether it changes.',
        range: 'Player drops range',
        ranges: { '24h': '24h', '7d': '7d', '30d': '30d', '90d': '90d' } satisfies Record<DropsRange, string>,
        over: {
            '24h': 'the last 24 hours',
            '7d': 'the last 7 days',
            '30d': 'the last 30 days',
            '90d': 'the last 90 days',
        } satisfies Record<DropsRange, string>,
        unit: 'per 100 h',
        rate: (rate: string) => `${rate} per 100 h`,
        unknown: 'n/a',
        playersCounted: 'Players counted',
        since: (day: string) => `since ${day}`,
        playersSince: (day: string) => `players counted since ${day}`,
        rates: 'Rates are drops per 100 player-hours, an hour of one player connected: a busier day does not raise them.',
        exportRates: 'Export rates',
        exportFailed: 'The export could not be made.',
        loading: 'Reading the player drops',
        failed: 'The player drops could not be read. They are read again in 30 s.',
        coverage: (closed: number, sessions: number, share: string, missed: number, lost: number) =>
            `Observer closed ${formatNumber('en', closed)} of ${count(sessions, { one: 'session', other: 'sessions' })} itself (${share}): ${formatNumber('en', missed)} left FXServer's player list without a drop, ${formatNumber('en', lost)} were still open when Observer stopped. How they ended is not known, so they stay out of the rates and the breakdown.`,
        categories: {
            quit: 'Quits',
            kick: 'Kicks',
            timeout: 'Timeouts',
            crash: 'Crashes',
            security: 'Security',
            serverStop: 'Server stops',
            serverCrash: 'Server crashes',
            unknown: 'Unknown',
        } satisfies Record<DropCategory, string>,
        categoriesInline: {
            quit: 'quits',
            kick: 'kicks',
            timeout: 'timeouts',
            crash: 'crashes',
            security: 'security drops',
            serverStop: 'server stops',
            serverCrash: 'server crashes',
            unknown: 'unknown drops',
        } satisfies Record<DropCategory, string>,
        kinds: {
            exit: 'Left the game',
            disconnect: 'Disconnected',
            otherServer: 'Joined another server',
            reconnect: 'Reconnected',
            editor: 'Opened the Rockstar Editor',
            reload: 'Reloaded the game',
            quitMessage: 'Quit with a message',
            resource: 'By a resource',
            connection: 'Connection lost',
            pendingCommands: 'Too many commands waiting',
            missedFrames: 'Missed frames',
            replaced: 'Replaced by a new connection',
            stateBagRate: 'Too many state bag changes',
            netEventRate: 'Too many net events',
            latentNetEventRate: 'Too many latent net events',
            commandRate: 'Too many commands',
            netGameEventRate: 'Too many game events',
            serverInitiated: 'Refused by the server',
            stop: 'Stopped',
            restart: 'Restarted',
            scheduled: 'Scheduled restart',
            shutdown: 'Observer stopped',
            crashLoop: 'Crash loop',
            console: 'Quit in the console',
            crash: 'Server crash',
            hang: 'Server stopped responding',
            lost: 'Lost when Observer stopped',
            missed: 'Missed',
            other: 'Other',
        } satisfies Record<DropKind, string>,
        verdicts: {
            rising: 'Rising',
            falling: 'Falling',
            noClearChange: 'No clear change',
            notEnoughDrops: 'Not enough drops',
        } satisfies Record<TrendVerdict, string>,
        verdict: {
            interval: (change: string, low: string, high: string) => `${change} (between ${low} and ${high})`,
            rates: (now: string, before: string) =>
                `Every drop: ${now} now, ${before} before, per 100 player-hours`,
            events: (now: number, before: number) =>
                `The test counts a burst of drops once: ${count(now, { one: 'event', other: 'events' })} now, ${formatNumber('en', before)} before`,
            notEnough: 'At least 20 events are needed to tell a change from chance.',
            noClearChange: "Within what this server's days vary by.",
        },
        windows: {
            today: (since: string, from: string, to: string) =>
                `Today since ${since} against yesterday from ${from} to ${to}.`,
            day: "Today since midnight against yesterday's same hours.",
            '7d': 'The last 7 days against the 7 days before.',
            '30d': 'The last 30 days against the 30 days before.',
            '90d': 'The last 90 days against the 90 days before.',
            change: 'The 72 hours before the change against the 72 hours after, cut at the next change.',
        },
        tiles: {
            labels: {
                crashes: 'Crashes',
                timeouts: 'Timeouts',
                kicks: 'Kicks',
                unexpected: 'Sessions ending unexpectedly',
            } satisfies Record<TileKind, string>,
            what: {
                crashes: 'crashes',
                timeouts: 'timeouts',
                kicks: 'kicks',
                unexpected: 'unexpected drops',
            } satisfies Record<TileKind, string>,
            ended: 'Ended this way',
            oneIn: (sessions: number) => `1 session in ${formatNumber('en', sessions)}`,
            drops: 'Drops',
            onePlayer: 'One player',
            onePlayerShare: (share: string, what: string) => `${share} of ${what}`,
            trend: (what: string, range: string) => `${what} per 100 player-hours over ${range}`,
        },
        chart: {
            title: 'Drop rate',
            category: 'Drops the chart shows',
            categories: {
                unexpected: 'Unexpected',
                crash: 'Crashes',
                timeout: 'Timeouts',
                kick: 'Kicks',
                security: 'Security',
            } satisfies Record<ChartCategory, string>,
            what: {
                unexpected: 'unexpected drops',
                crash: 'crashes',
                timeout: 'timeouts',
                kick: 'kicks',
                security: 'security drops',
            } satisfies Record<ChartCategory, string>,
            every: (step: string) => `Every ${step} · per 100 player-hours`,
            rate: 'Rate',
            baseline: '4-week baseline',
            drops: 'Drops',
            tooltipRate: (rate: string) => `${rate} per 100 h`,
            tooltipNobody: 'Nobody played',
            tooltipDrops: (drops: number) => count(drops, { one: 'drop', other: 'drops' }),
            tooltipBaseline: (rate: string) => `baseline ${rate}`,
            summary: (drops: number, what: string, range: string, rate: string | null) =>
                rate === null
                    ? `${formatNumber('en', drops)} ${what} over ${range}.`
                    : `${formatNumber('en', drops)} ${what} over ${range}, ${rate} per 100 player-hours.`,
            peak: (rate: string, at: string) => `Highest ${rate} per 100 h, at ${at}.`,
            baselineSpan: (low: string, high: string) =>
                low === high
                    ? `The 4-week baseline is ${low} per 100 h.`
                    : `The 4-week baseline runs from ${low} to ${high} per 100 h.`,
            noBaseline: 'No 4-week baseline yet: the weeks before are not known.',
            unplayed: (buckets: number, total: number) =>
                `Nobody played in ${formatNumber('en', buckets)} of its ${formatNumber('en', total)} buckets: the line breaks there.`,
            empty: 'Nobody played over this range.',
            loading: 'Loading the drop rate chart',
            failed: 'The drop rate chart could not be read. It is read again in 30 s.',
        },
        breakdown: {
            title: 'Breakdown',
            meta: (sessions: number, played: string) =>
                `${count(sessions, { one: 'session', other: 'sessions' })} ended · ${played} played`,
            caption: 'Drops by category over the range, unexpected ones first, most drops first',
            category: 'Category',
            drops: 'Drops',
            share: 'Share of all drops',
            rate: 'Per 100 h',
            verdict: 'Trend',
            loading: 'Reading the breakdown',
            loadingShare: (share: string) => `${share} while loading`,
            unexpected: 'Unexpected',
            expected: 'Expected',
            notPlayers: 'Not a player rate',
            filterHint:
                'Pick a category to narrow the log to it. Crashes, timeouts, kicks and security narrow the chart too; unknown drops show it as unexpected ones.',
            filterHintChart:
                'Pick crashes, timeouts, kicks or security to narrow the chart to them; unknown drops show it as unexpected ones.',
            empty: 'No drop over this range.',
            failed: 'The breakdown could not be read.',
        },
        signatures: {
            title: 'Crash signatures',
            meta: (crashes: number) => count(crashes, { one: 'crash', other: 'crashes' }),
            groupBy: 'Group crashes by',
            tabs: { signatures: 'Signatures', modules: 'Modules', builds: 'Game builds' },
            caption: 'Crash signatures over the range, most crashes first',
            modulesCaption: 'Crashes by the module they happened in, most crashes first',
            buildsCaption: 'Crashes by game build, most crashes first',
            signature: 'Signature',
            module: 'Module',
            build: 'Game build',
            crashes: 'Crashes',
            share: 'Share of crashes',
            players: 'Players',
            firstSeen: 'First seen',
            verdict: 'Trend',
            signaturesCount: 'Signatures',
            new: 'New',
            newHint: 'First seen in this range, after 28 days without it.',
            hidden: 'Hidden until a second player crashes with it.',
            hiddenHint:
                'A client can write anything after its crash: a text one player alone wrote is not shown.',
            gameBuild: (build: number) => `game build ${build}`,
            playersCount: (players: number) => count(players, { one: 'player', other: 'players' }),
            forms: {
                hash: 'Code name',
                symbol: 'Symbol',
                error: 'Error code',
                load: 'Asset load',
                exception: 'Exception',
                text: 'Text',
            } satisfies Record<CrashForm, string>,
            asset: (resource: string) => `while loading an asset of ${resource}`,
            examples: 'Examples',
            hideExamples: 'Hide the examples',
            examplesTitle: 'Latest crash texts, identifiers taken out',
            noExamples: 'No crash text is kept for it.',
            noSharedExample: 'No example shared by two players yet.',
            loadingShare: (share: string) => `${share} of these crashes happened while loading.`,
            noModule: 'No module named',
            noBuild: 'No game build named',
            showInLog: 'Show in the log',
            empty: 'No crash over this range.',
            loading: 'Reading the crash signatures',
            failed: 'The crash signatures could not be read.',
            repeat: (players: number) =>
                `${count(players, { one: 'player', other: 'players' })} crashed 3 times or more in the last 7 days.`,
            repeatNamed: (names: ReactNode, others: number): ReactNode => [
                'Crashed 3 times or more in the last 7 days: ',
                names,
                others > 0 ? `, and ${count(others, { one: 'other player', other: 'other players' })}.` : '.',
            ],
            repeatCrashes: (crashes: number) => count(crashes, { one: 'crash', other: 'crashes' }),
        },
        resources: {
            title: 'Kicking resources',
            meta: (kicks: number) => count(kicks, { one: 'kick', other: 'kicks' }),
            caption: 'Resources that kicked players over the range, most kicks first',
            resource: 'Resource',
            kicks: 'Kicks',
            share: 'Share of kicks',
            players: 'Players',
            verdict: 'Trend',
            noResource: 'No resource named',
            reasons: 'Reasons',
            hideReasons: 'Hide the reasons',
            reasonsCaption: (resource: string) => `What ${resource} gave as its reasons, most kicks first`,
            reasonsNote: 'Numbers, names and quoted words are taken out, so alike reasons group.',
            reasonKicks: (kicks: number) => count(kicks, { one: 'kick', other: 'kicks' }),
            playersCount: (players: number) => count(players, { one: 'player', other: 'players' }),
            noReason: 'No reason is kept for these kicks.',
            showInLog: 'Show in the log',
            empty: 'No resource kicked a player over this range.',
            loading: 'Reading the kicking resources',
            failed: 'The kicking resources could not be read.',
        },
        changes: {
            title: 'Changes and incidents',
            caption: 'Restarts, build changes, server crashes and incidents over the range, newest first',
            chips: {
                restart: 'Restart',
                build: 'Build',
                serverCrash: 'Crash',
                hang: 'Hang',
                incident: 'Incident',
            },
            restart: 'FXServer started.',
            build: (from: number | null, to: number | null) =>
                from === null || to === null
                    ? `FXServer started on build ${to ?? from ?? '?'}.`
                    : `FXServer went from build ${from} to ${to}.`,
            crash: (players: number | null) =>
                players === null
                    ? 'FXServer crashed.'
                    : `FXServer crashed, dropping ${count(players, { one: 'player', other: 'players' })}.`,
            hang: (players: number | null) =>
                players === null
                    ? 'FXServer stopped responding.'
                    : `FXServer stopped responding, dropping ${count(players, { one: 'player', other: 'players' })}.`,
            incident: (drops: number | null, hitch: string | null) => {
                const what =
                    drops === null ? 'Players dropped' : count(drops, { one: 'drop', other: 'drops' });
                return hitch === null ? `${what} within 10 s.` : `${what} during a ${hitch} hitch.`;
            },
            incidentCategory: (drops: number, category: string) => `${formatNumber('en', drops)} ${category}`,
            online: (players: number) => `${count(players, { one: 'player', other: 'players' })} online`,
            after: (category: string, before: string, after: string) =>
                `After it: ${category} ${before} to ${after} per 100 h`,
            newSignatures: (signatures: number, named: ReactNode): ReactNode => [
                `${count(signatures, { one: 'new signature', other: 'new signatures' })} after it: `,
                named,
            ],
            seconds: (seconds: string) => `${seconds} s`,
            run: (run: number) => `run ${run}`,
            showInLog: 'Show their drops',
            snapshots:
                'Resource, cfg and file changes join this timeline once file changes is installed. Until then it holds restarts, FXServer builds, server crashes and incidents.',
            empty: 'No restart, build change, server crash or incident over this range.',
            loading: 'Reading the changes',
            failed: 'The changes could not be read.',
        },
        uptime: {
            title: 'Uptime',
            meta: 'Unexpected drops per 100 h, by time since the start',
            bin: (from: number, to: number | null) => (to === null ? `${from}h+` : `${from}-${to}h`),
            higher: 'Higher',
            higherBins: (bins: string[]) => `${list(bins)}, against the other hours since a start.`,
            notEnough: 'Not enough drops',
            summary: (bins: readonly string[]) =>
                `Unexpected drops per 100 player-hours, a burst counting once, by hours since FXServer started: ${bins.join(', ')}.`,
            summaryBin: (bin: string, rate: string, verdict: string | null) =>
                verdict === null ? `${bin} ${rate}` : `${bin} ${rate} (${verdict})`,
            note: 'A burst of drops counts once. A rate that climbs with the hours since the start points to a leak or a resource that degrades.',
            since: (day: string) => `Sessions counted since ${day}: older ones are no longer kept.`,
            empty: 'Nobody played over this range.',
            loading: 'Reading the uptime figures',
            failed: 'The uptime figures could not be read.',
        },
        log: {
            title: 'Drop log',
            meta: (range: string) => `Over ${range}, newest first`,
            filters: 'Filter the log',
            category: 'Category',
            anyCategory: 'Any category',
            kind: 'Kind',
            anyKind: 'Any kind',
            resource: 'Resource',
            signature: 'Crash signature',
            anySignature: 'Any signature',
            signatureId: (id: number) => `Signature ${id}`,
            player: 'Player',
            playerHint: 'Player: the start of a name, or a licence.',
            playerHintIdentifiers:
                'Player: the start of a name, a licence, or any identifier with its prefix, such as discord:. Never an address. A player typed takes the place of a profile kept.',
            run: 'Run',
            runInvalid: 'A run is a number, as the log shows it.',
            apply: 'Apply',
            clear: 'Clear',
            export: 'Export CSV',
            profile: (id: number) => `Player ${id}`,
            group: (id: number) => `Player ${id} and their linked licences`,
            removeFilter: (filter: string) => `Remove the filter: ${filter}`,
            caption: 'Drops over the range under the filters, newest first',
            time: 'Time',
            playerColumn: 'Player',
            categoryColumn: 'Category',
            detail: 'Detail',
            session: 'Session',
            phase: 'Phase',
            runColumn: 'Run',
            unconfirmed: 'Unconfirmed',
            unconfirmedHint: 'Who this was was never confirmed, or the player was forgotten.',
            noPlayer: 'Not known',
            loadingPhase: 'Loading',
            inGame: 'In game',
            byResource: (resource: ReactNode): ReactNode => ['by ', resource],
            closedByObserver: 'Closed by Observer',
            duringIncident: 'During an incident',
            runValue: (run: number) => `Run ${run}`,
            filterRun: (run: number) => `Show run ${run} only`,
            filterSignature: (signature: string) => `Show the signature ${signature} only`,
            empty: 'No drop over this range.',
            noMatch: 'No drop matches these filters over this range.',
            loadMore: 'Load more',
            shown: (drops: number) => `${count(drops, { one: 'drop', other: 'drops' })} shown`,
            allShown: (drops: number) => `All ${count(drops, { one: 'drop', other: 'drops' })} shown`,
            reading: 'Reading the drop log',
            failed: 'The drop log could not be read.',
        },
        settings: {
            title: 'Drop data',
            meta: 'Owner only',
            lead: "How long Observer keeps each player's sessions, with their crash and kick texts. The hourly figures behind the charts and the rates stay.",
            sessionDays: 'Keep sessions for',
            hint: 'Days, from 14 to 365.',
            outOfRange: 'Out of range. Days, from 14 to 365.',
            changes: (changes: number) => `${count(changes, { one: 'change', other: 'changes' })} not saved`,
            discard: 'Discard',
            discarded: 'Changes discarded.',
            save: 'Save the retention',
            saved: 'Retention saved. Older sessions go within the hour.',
            loading: 'Reading the retention',
            failed: 'The retention could not be read.',
        },
        overview: {
            label: 'Player drops · 24h',
            failed: 'The player drops could not be read. They are read again in 30 s.',
            ended: 'Ended unexpectedly',
            drops: 'Unexpected drops',
            trend: 'Unexpected drops per 100 player-hours over the last 24 hours',
            open: 'Open Player drops',
        },
        crash: {
            dropped: (players: number) => `${count(players, { one: 'player', other: 'players' })} dropped`,
            showInLog: 'Show their drops',
        },
        profileLog: 'Open in the drop log',
    },
    changes: {
        title: 'Changes',
        gloss: 'What each start of the server ran from.',
        timeNote: 'Times in your time zone',
        loading: 'Reading the changes',
        empty: {
            title: 'No start recorded yet',
            body: 'Snapshots start with the next start of the server: each one records the build, the cfg and every resource it runs from.',
            baselineTitle: 'Baseline taken',
            baselineBody:
                'The first start is the baseline: it records every resource as found. What changes shows from the next start.',
            latest: 'Compare the latest starts',
        },
        bar: {
            title: 'Compared',
            from: 'From',
            to: 'To',
            disk: 'On disk now',
            diskAt: (time: string) => `On disk now · checked ${time}`,
            diskChecking: 'On disk now · checking',
            check: 'Check the disk',
            checking: 'Checking the disk',
            checkHint: 'Lists and hashes the server folder now: what the next start would apply.',
            option: (time: string, start: string, changes: string) => `${time} · ${start} · ${changes}`,
        },
        causes: {
            requested: 'Started',
            restart: 'Restarted',
            scheduled: 'Scheduled restart',
            recovery: 'Recovered',
        } satisfies Record<StartCause, string>,
        startBy: (cause: string, by: string) => `${cause} by ${by}`,
        start: 'Start',
        check: 'Disk check',
        states: {
            listing: 'Listing files',
            hashing: 'Hashing files',
            complete: 'Complete',
            partial: 'Partial',
            failed: 'Failed',
        } satisfies Record<SnapshotState, string>,
        partialReasons: {
            limit: 'it reached the limits of a snapshot',
            cancelled: 'the next start cut it short',
            diskFloor: 'the disk was almost full, so no contents were kept',
            unreadable: 'some files could not be read',
        } satisfies Record<PartialReason, string>,
        partial: (reason: string) =>
            `This snapshot is partial: ${reason}. What it could not read shows as Unknown.`,
        failed: 'This snapshot failed: the server folder could not be listed.',
        busy: {
            listing: 'The files are still being listed: the counts change as they are read.',
            hashing: 'The files are still being hashed: resources marked Checking settle as they are read.',
        },
        counted: {
            added: (value: number) => `${formatNumber('en', value)} added`,
            removed: (value: number) => `${formatNumber('en', value)} removed`,
            updated: (value: number) => `${formatNumber('en', value)} updated`,
            sameVersion: (value: number) => `${formatNumber('en', value)} same version`,
            renamed: (value: number) => `${formatNumber('en', value)} renamed`,
            moved: (value: number) => `${formatNumber('en', value)} moved`,
            didNotStart: (value: number) => `${formatNumber('en', value)} did not start`,
            checking: (value: number) => `${formatNumber('en', value)} checking`,
        },
        resourcesChanged: (value: number) =>
            count(value, { one: 'resource changed', other: 'resources changed' }),
        // Resources neither changed nor unchanged yet, or left so by a partial snapshot.
        beingChecked: (value: number) =>
            `${count(value, { one: 'resource', other: 'resources' })} being checked`,
        leftUnknown: (value: number) => `${count(value, { one: 'resource', other: 'resources' })} unknown`,
        noChange: 'No change',
        baseline: 'Baseline',
        beingListed: 'Being listed',
        snapshotFailed: 'Snapshot failed',
        notCompared: 'No earlier start to compare with',
        build: (build: number) => `build ${build}`,
        lines: (added: string, removed: string) => `+${added} −${removed}`,
        files: (added: string, modified: string, removed: string) => `+${added} ~${modified} −${removed}`,
        fromTo: (from: string, to: string) => `${from} → ${to}`,
        summary: {
            label: 'What changed',
            resources: 'Resources',
            files: 'Files',
            build: 'Build',
            gameBuild: 'Game build',
            cfg: 'cfg',
            launch: 'Launch settings',
            same: 'Same',
            unchanged: (value: number) =>
                `${count(value, { one: 'resource', other: 'resources' })} unchanged`,
            baselineSub: 'first snapshot, every resource as found',
            filesSub: 'added, removed or modified',
            whileStarting: (value: number) =>
                `${count(value, { one: 'file', other: 'files' })} changed while the server was starting`,
            from: (value: string) => `from ${value}`,
            cfgFiles: (value: number) => count(value, { one: 'file', other: 'files' }),
            settings: (value: number) => count(value, { one: 'change', other: 'changes' }),
        },
        environment: {
            title: 'Environment',
            meta: 'What the server ran with besides its resources',
            same: 'Same build, game build, launch settings and cfg.',
            build: 'Build',
            gameBuild: 'Game build',
            game: 'Game',
            settings: 'Launch settings',
            arguments: 'Extra arguments',
            cfg: 'cfg',
            cfgStarted: 'Started by the cfg now',
            cfgStopped: 'No longer started by the cfg',
            cfgCount: (value: number) => `${count(value, { one: 'file', other: 'files' })} changed`,
            settingNames: {
                onesync: 'OneSync',
                autoStart: 'Start with Observer',
                txadminEvents: 'txAdmin events',
                cfgFile: 'Config file',
                dataPath: 'Server data folder',
            } as Record<string, string>,
            argumentChanges: {
                set: 'set',
                removed: 'removed',
                changed: 'value changed',
            } satisfies Record<ArgumentChangeKind, string>,
            attribution: (action: string, by: string, at: string) => `${action} by ${by} · ${at}`,
            observer: 'Observer',
            showDiff: 'Show the diff',
            hideDiff: 'Hide the diff',
            notSet: 'not set',
            yes: 'on',
            no: 'off',
        },
        resources: {
            title: 'Resources',
            meta: (value: number) => count(value, { one: 'resource changed', other: 'resources changed' }),
            filters: 'Changes shown',
            search: 'Search by name or path',
            caption: 'Resources that changed, those changed under the same version first',
            resource: 'Resource',
            change: 'Change',
            version: 'Version',
            files: 'Files',
            lines: 'Lines',
            by: 'By',
            kinds: {
                added: 'Added',
                removed: 'Removed',
                renamed: 'Renamed',
                moved: 'Moved',
                updated: 'Updated',
                sameVersion: 'Same version',
                checking: 'Checking',
                unknown: 'Unknown',
                didNotStart: 'Did not start',
                unchanged: 'Unchanged',
            } satisfies Record<ResourceChangeKind, string>,
            escrowed: 'Escrowed',
            duplicate: 'Duplicate',
            fromPath: (path: string) => `from ${path}`,
            filesToo: 'files changed too',
            none: 'No resource changed between these two snapshots.',
            noMatch: 'No resource matches the search and the changes shown.',
            baseline:
                'This start is a baseline: it lists every resource as found, so none of them counts as added.',
        },
        live: {
            title: 'While the server ran',
            caption: 'Resources started, stopped or restarted while the later start ran',
            kinds: {
                started: 'Started',
                stopped: 'Stopped',
                restarted: 'Restarted',
            } satisfies Record<LiveChangeKind, string>,
            by: (name: string) => `by ${name}`,
            byUnknown: 'by unknown',
            showAll: (value: number) => `Show all ${formatNumber('en', value)}`,
            showFewer: 'Show fewer',
        },
        starts: {
            title: 'Starts',
            meta: 'Pick one to compare it with the start before',
            caption: 'Starts, newest first',
            compare: (time: string) => `Compare the start of ${time} with the one before`,
            compared: 'Compared',
            crashed: 'Crashed',
            loadMore: 'Load older starts',
        },
        facts: {
            build: 'build',
            gameBuild: 'game build',
            launch: 'launch settings',
            cfg: 'cfg',
        } satisfies Record<EnvironmentFact, string>,
        resource: {
            back: 'All changes',
            loading: 'Reading the resource',
            version: 'Version',
            noVersion: 'no version',
            path: 'Folder',
            git: 'Git',
            detached: 'detached',
            packages: 'npm packages',
            packageAdded: (version: string) => `added ${version}`,
            packageRemoved: (version: string) => `removed, was ${version}`,
            duplicate: (paths: string[]) =>
                `FXServer loads only one folder of this name and warns about the others. Also found: ${list(paths)}.`,
            files: 'Files',
            showUnchanged: 'Show unchanged files',
            root: 'At the root',
            noFile: 'No file changed in this resource.',
            pickFile: 'Pick a file to see what changed in it.',
            pickFileNoContent:
                'Pick a file to see its sizes, hashes and who wrote it. Its lines take the permission to read changed files.',
            noContent:
                'Your account sees which files changed, not their contents: reading them takes the permission to read changed files.',
            sizes: (from: string, to: string) => `${from} → ${to}`,
            writtenBy: (by: string, at: string) => `written over SFTP by ${by} at ${at}`,
            purge: (name: string) => `Purge the kept contents of ${name}`,
            purgeTitle: 'Purge the kept contents?',
            purgeBody:
                "Every kept version of this file leaves Observer's history, in every start and check. Its changes stay listed, without their lines. Add a never-show pattern so that it is not kept again.",
            purgeConfirm: 'Purge contents',
            purged: (path: string, versions: number, bytes: string) =>
                `${path}: ${count(versions, { one: 'version', other: 'versions' })} cleared, ${bytes}.`,
            history: 'History',
            historyMeta: 'Each version kept over the starts',
            historyCaption: 'Versions of the resource, newest first',
            since: (time: string) => `since ${time}`,
            historyEmpty: 'No version of this resource is kept.',
        },
        diff: {
            label: (path: string) => `Changes in ${path}`,
            lines: (path: string) => `Lines of ${path}`,
            layout: 'Layout',
            unified: 'Unified',
            split: 'Split',
            ignoreWhitespace: 'Ignore whitespace',
            wrap: 'Wrap lines',
            previous: 'Previous file',
            next: 'Next file',
            viewFile: 'View file',
            viewDiff: 'View changes',
            side: 'Version shown',
            before: 'Before',
            after: 'After',
            added: 'added',
            removed: 'removed',
            hiddenSecret: 'Hidden secret',
            redaction: '[REDACTED]',
            secretChanged: 'secret changed',
            invisible: {
                bidi: (code: string) => `bidirectional control ${code}`,
                zeroWidth: (code: string) => `zero-width character ${code}`,
            } satisfies Record<InvisibleKind, (code: string) => string>,
            invisibleWarning:
                'This file holds characters that reorder or hide code, as a Trojan Source attack uses them. Each one shows as ⟨U+…⟩ in its place.',
            moreLines: 'Show 20 more lines',
            page: (page: number, pages: number) =>
                `Page ${formatNumber('en', page)} of ${formatNumber('en', pages)}`,
            previousPage: 'Previous page',
            nextPage: 'Next page',
            loading: 'Reading the lines',
            noLine: 'No line changed.',
            lineEndings: 'Line endings changed, nothing else.',
            unchanged: 'This file did not change.',
            binary: 'Binary',
            sameSize: 'same size, contents changed',
            encrypted: 'Encrypted by the Cfx.re escrow',
            encryptedChanged: 'Encrypted by the Cfx.re escrow · changed',
            hidden: 'Contents never shown: matches',
            generated: 'Generated or minified: compared by size and hash.',
            notKept: 'Contents no longer kept: the change stays listed, without its lines.',
            beingRead: 'Still being read: its lines show once the files of this snapshot are hashed.',
            renamed: (from: string) => `Renamed from ${from}; contents identical.`,
            archive: 'Archive: compared by size and time, never read.',
            offline: 'Cloud placeholder: listed by size and time, never read.',
            link: 'Link: compared by where it points.',
            unreadable: 'This file could not be read.',
            flags: {
                afterStart:
                    'Changed while the server was starting: the running server may hold the earlier version.',
                unsettled: 'Changed while it was being read: no contents kept for this version.',
                reloadedLive: 'Reloaded live after it changed: the running server holds this version.',
            } satisfies Record<FileFlag, string>,
            flagNames: {
                afterStart: 'changed while starting',
                unsettled: 'changed while read',
                reloadedLive: 'reloaded live',
            } satisfies Record<FileFlag, string>,
            shaBefore: 'SHA-256 before',
            shaAfter: 'SHA-256 after',
            fileKinds: {
                text: 'Text',
                generated: 'Generated',
                encrypted: 'Encrypted',
                binary: 'Binary',
                hidden: 'Hidden',
                archive: 'Archive',
                offline: 'Placeholder',
                link: 'Link',
                unreadable: 'Unreadable',
            } satisfies Record<FileKind, string>,
            fileChanges: {
                added: 'Added',
                removed: 'Removed',
                renamed: 'Renamed',
                modified: 'Modified',
                unchanged: 'Unchanged',
            } satisfies Record<FileChangeKind, string>,
        },
        settings: {
            title: 'Change tracking',
            meta: 'Owner only',
            loadFailed: 'The settings could not be loaded.',
            enabled: 'Take a snapshot at each start',
            enabledHint: 'Lists the server folder as FXServer starts and hashes what changed once it is up.',
            extensions: 'Extensions with line diffs',
            extensionsHint:
                'Separated by spaces or commas, without the dot. Other files are compared by size and hash.',
            extensionsInvalid:
                'Each extension is 1 to 16 letters, digits, dashes or underscores, 200 at most.',
            excluded: 'Never walked',
            excludedHint:
                'One pattern a line. * and ? are wildcards; a pattern with / is a path from the server data folder, one without names a file or folder anywhere.',
            hidden: 'Contents never kept',
            hiddenHint: 'Files compared by digest only: their lines are never stored nor shown.',
            matches: (value: number) => `matches ${count(value, { one: 'file', other: 'files' })}`,
            matchesNone: 'matches no file',
            maxText: 'Largest text kept',
            maxTextHint: 'In MB, up to 8. A larger text file is compared by digest only.',
            budget: 'Contents kept at most',
            budgetHint: 'In GB. Past it, the oldest contents go first; the last two starts stay whole.',
            stored: (used: string) => `${used} kept now`,
            retentionDays: 'Days a start is kept',
            hashRate: 'Hashing rate',
            hashRateHint: 'In MB/s while the server runs.',
            verifyDays: 'Days between full verifications',
            verifyDaysHint:
                '0 for none. A full verification hashes every file again, which catches a copy that kept both size and time.',
            startEvent: 'Raise observer:startChanges',
            startEventHint: 'Once per run, for resources that post a changelog or tell the staff.',
            outOfRange: 'Out of range.',
            changes: (value: number) =>
                `${count(value, { one: 'unsaved change', other: 'unsaved changes' })}`,
            discard: 'Discard',
            save: 'Save',
            saved: 'Settings saved.',
            verify: 'Verify all files',
            verifyStarted: 'Verification started: every file is hashed again in the background.',
        },
        overview: {
            since: (parts: string) => `Changes since the previous start: ${parts}`,
            resources: (value: number) => count(value, { one: 'resource', other: 'resources' }),
            none: 'No change since the previous start',
            baseline: 'Baseline being taken',
            baselineTaken: 'Baseline taken: changes show from the next start',
            listing: 'Changes since the previous start: being listed',
            failed: 'The snapshot of the latest start failed',
            notCompared: 'No earlier start to compare the latest with',
        },
    },
    tools: {
        title: 'Tools',
        gloss: 'Problems in the server folder that FXServer does not report.',
        loadFailed: 'The tools could not be loaded.',
        stopFirst: 'Stop FXServer to fix or undo: the tools change files only while it is stopped.',
        stateUnknown:
            "Fix and Undo wait for FXServer's state, which is not known while the live link is down.",
        archetypes: {
            title: 'Duplicate archetypes',
            about: 'The FiveM client logs a Duplicate Archetype warning when two files it loads define the same prop, vehicle, weapon or ped model. By default the tool fixes only identical copies within one resource. It keeps the original of every file it changes, so each fix can be undone.',
            scan: 'Scan now',
            fix: 'Fix',
            fixPlan: (groups: number, removals: number, files: number) =>
                `Fix takes ${count(removals, { one: 'copy', other: 'copies' })} out of ${count(files, { one: 'file', other: 'files' })}, for ${count(groups, { one: 'duplicate', other: 'duplicates' })}.`,
            noReport: 'Scan first: Fix applies the plan of the last report.',
            nothingToFix: 'The last report has nothing to fix.',
            applied: "The last report's fixes are written. Scan now to see what is left.",
            outdated:
                'Files were put back since the last report, which does not know of them. Scan now to see what is fixable again.',
            scanned: (groups: number, fixable: number) =>
                groups === 0
                    ? 'Scan done: no archetype is registered more than once.'
                    : `Scan done: ${count(groups, { one: 'duplicate', other: 'duplicates' })}, ${formatNumber('en', fixable)} of them fixable.`,
            fixed: (groups: number, files: number) =>
                `Fixed ${count(groups, { one: 'duplicate', other: 'duplicates' })} in ${count(files, { one: 'file', other: 'files' })}. FXServer loads them the next time it starts their resources.`,
            ended: (outcome: string, problem: string | null) =>
                problem === null ? `${outcome}.` : `${outcome}: ${problem}`,
            lastRun: 'Last run',
            noRun: 'No run yet. Scan now reads the server folder and writes nothing.',
            when: 'When',
            by: 'Run by',
            mode: 'Run',
            outcome: 'Outcome',
            took: (time: string, took: string) => `${time}, took ${took}`,
            triggers: {
                start: 'Observer, before a start of FXServer',
                panel: 'The Tools page',
                cli: 'The command line',
            } satisfies Record<ToolTrigger, string>,
            byActor: (actor: string) => `${actor}, on the Tools page`,
            modes: { report: 'Report', fix: 'Fix', heal: 'Heal', undo: 'Undo' } satisfies Record<
                ToolRunMode,
                string
            >,
            outcomes: {
                done: 'Done',
                cancelled: 'Cancelled',
                timedOut: 'Timed out',
                failed: 'Failed',
            } satisfies Record<ToolRunOutcome, string>,
            running: 'Running',
            summary: {
                groups: 'Duplicates',
                fixable: 'Fixable',
                fixed: 'Fixed',
                left: 'Left',
                filesChanged: 'Files written',
                removals: 'Copies taken out',
                filesRestored: 'Files put back',
                openChanges: 'Open changes',
            },
            reportFrom: (time: string) =>
                `The groups below are from the report of ${time}: the last run made none.`,
        },
        groups: {
            title: 'Groups',
            meta: (total: number) => count(total, { one: 'duplicate', other: 'duplicates' }),
            noReport: 'No report yet: Scan now lists every archetype registered more than once.',
            none: 'No archetype is registered more than once.',
            search: 'Search names, hashes, resources and files',
            actionFilters: 'Filter by action',
            reasonFilters: 'Filter by reason',
            noMatch: 'No group matches the search and filters.',
            caption: 'Archetypes registered more than once, with their copies and what the tool does',
            archetype: 'Archetype',
            copies: 'Copies',
            difference: 'Difference',
            action: 'Action',
            sameFile: 'one file',
            escrowed: 'escrowed',
            kept: 'kept',
            removed: 'taken out',
            entry: (index: number) => `entry #${index}`,
            archetypeTypes: { base: null, time: 'time archetype', mlo: 'interior' } satisfies Record<
                ArchetypeType,
                string | null
            >,
            item: (kind: string, position: number, line: number) =>
                `${kind} item #${position}, line ${formatNumber('en', line)}`,
            modelInfoKinds: { weapon: 'weapon', vehicle: 'vehicle', ped: 'ped' } satisfies Record<
                ModelInfoKind,
                string
            >,
            differences: {
                identical: 'Identical',
                minor: 'Draw distances and bounds',
                major: 'Differs',
            } satisfies Record<Difference['kind'], string>,
            actions: { fixed: 'Fixed', toFix: 'To fix', left: 'Left' },
            keeps: (kept: number, removed: number[]) =>
                `Keeps copy ${kept}, takes out ${removed.length === 1 ? 'copy' : 'copies'} ${list(removed.map(String))}.`,
            reasons: {
                definitionsDiffer: 'Copies differ',
                severalResources: 'Several resources',
                escrowed: 'Escrowed',
                interior: 'Interior',
                mixedSources: '.ytyp and .meta',
                contested: 'Name contested',
                manifestDependent: 'Listed by a manifest',
                notWritable: 'Cannot be changed',
                excluded: 'Excluded',
            } satisfies Record<LeaveReason['kind'], string>,
            reason: (reason: LeaveReason) => leaveReasonText(reason),
            shown: (shown: number, total: number) =>
                shown === total
                    ? `${count(total, { one: 'group', other: 'groups' })} shown`
                    : `${formatNumber('en', shown)} of ${count(total, { one: 'group', other: 'groups' })} shown`,
            showMore: 'Show more',
            truncated: (listed: number, total: number) =>
                `The report lists the first ${formatNumber('en', listed)} of ${count(total, { one: 'group', other: 'groups' })}.`,
        },
        report: {
            label: 'What the scan read',
            resources: (found: number, started: number, client: number, escrowed: number) =>
                `${count(found, { one: 'resource', other: 'resources' })}: ${formatNumber('en', started)} predicted to start, ${formatNumber('en', client)} sent to clients, ${formatNumber('en', escrowed)} of those escrowed`,
            ytyp: (loaded: number, shadowed: number, unreadable: number) =>
                `.ytyp files: ${formatNumber('en', loaded)} loaded, ${formatNumber('en', shadowed)} shadowed, ${formatNumber('en', unreadable)} unreadable`,
            modelInfo: (loaded: number, unreadable: number) =>
                `Model-info .meta files: ${formatNumber('en', loaded)} loaded, ${formatNumber('en', unreadable)} unreadable`,
            registrations: (value: number) =>
                count(value, { one: 'archetype registered', other: 'archetypes registered' }),
            took: (time: string) => `read in ${time}`,
            shadowed: (value: number) => `Shadowed files (${formatNumber('en', value)})`,
            shadowedHint: 'Clients load another file of the same name in their place.',
            shadowedBy: (path: string, resource: string) => `loaded instead: ${path} in ${resource}`,
            unreadable: (value: number) => `Unreadable files (${formatNumber('en', value)})`,
            unreadableHint: 'The archetypes these files define are unknown.',
            problem: (problem: FileProblem) => fileProblemText(problem),
            notes: (value: number) => `Notes (${formatNumber('en', value)})`,
            notesHint:
                'What limits the prediction of which resources start and which files the clients load.',
            noteKinds: {
                orderNotGuaranteed:
                    'This system lists folders in no set order: which of two stream files of the same name FXServer sends may differ from the prediction.',
                cfgIncomplete:
                    'A file the cfg runs was not read: the resources it starts or stops are missing.',
                allResources:
                    'No cfg was read: every resource FXServer accepts is taken as started, in name order.',
                unsupportedGame: 'The cfg runs another game than GTA V: the tool does nothing there.',
                missingResource: 'The cfg names a resource FXServer does not find.',
                missingDependency: 'A resource depends on one FXServer does not find, and does not start.',
                dependencyNotStarted: "A resource's dependency does not start, so neither does it.",
                constraintNotChecked:
                    'A dependency on the server itself, which only FXServer decides, is taken as met.',
                duplicateResourceName:
                    'Two resource folders have the same name: FXServer uses the first one found.',
                noManifest: 'A resource folder has no fxmanifest.lua or __resource.lua, and never starts.',
                manifestRefused: 'FXServer refuses this manifest, so the resource never starts.',
                manifestEncrypted:
                    'The manifest is encrypted by the asset escrow: its dependencies and data files are unknown.',
                manifestUnreadable: 'The manifest could not be read.',
                manifestNote: 'The manifest uses something the tool does not follow.',
                rpfInStream: 'An .rpf in stream/, which the client refuses.',
                rpfDataFile:
                    'An RPF_FILE data file, whose archive may define archetypes the tool does not see.',
                dataFileMissing: 'A model-info data file names no file the resource ships.',
                dataFileMismatch: "A model-info data file's type differs from the file's own.",
                folderTooLarge:
                    'A folder holds more entries than the tool lists: what lies past them is unknown.',
                referenceUnreadable:
                    'A .ymf manifest or _cache_y.dat could not be read: the files it names are unknown.',
                baseGameUnseen:
                    "The archetypes of GTA V itself and of Rockstar's updates are not read: a resource that defines one of them again is not seen.",
            } satisfies Record<ToolNoteKind, string>,
            found: (times: number) => `found ${count(times, { one: 'time', other: 'times' })}`,
            more: (value: number) => `and ${formatNumber('en', value)} more`,
        },
        changes: {
            title: 'Files changed',
            backups: (bytes: string) => `Backups take ${bytes}`,
            none: 'No file is changed. Each fix shows here, with the backup of the file as it was.',
            caption: 'Files the tool changed, with the copies taken out and their backups',
            file: 'File',
            entries: 'Copies taken out',
            backup: 'Backup',
            changed: 'Changed',
            actions: 'Undo',
            copyBackup: (path: string) => `Copy the backup's name of ${path}, under data/tools/backups`,
            pending: 'Being written',
            notYet: 'not yet',
            keptHere: (index: number) => `kept: #${index} in this file`,
            keptIn: (path: string, index: number) => `kept: ${path} #${index}`,
            showAll: (value: number) => `Show all ${formatNumber('en', value)}`,
            showFewer: 'Show fewer',
            undo: 'Undo',
            undoAll: 'Undo all',
            undoFile: (path: string) => `Undo the change of ${path}`,
            runs: 'Open changes by run',
            run: (id: number, files: number, removals: number) =>
                `Run ${id}: ${count(files, { one: 'file', other: 'files' })}, ${count(removals, { one: 'copy', other: 'copies' })} taken out`,
            undoRun: (id: number) => `Undo run ${id}`,
            elsewhere:
                "Some files lie in another server folder than the launch settings'. Undo all leaves them: undo them by file or by run, in their own folder.",
            unreachable:
                'Its folder is gone, or is another folder now: no undo reaches it. Copy its backup back by hand.',
            undone: (files: number) =>
                files === 0
                    ? 'Nothing was put back.'
                    : `Put back ${count(files, { one: 'file', other: 'files' })}. FXServer loads them the next time it starts their resources.`,
            closed: 'Recently closed',
            closedCaption: 'Changes closed in the last 30 days, the latest first',
            closeReasons: {
                undone: 'Undone',
                healed: 'Healed',
                stale: 'Changed by someone else',
                restored: 'Put back outside Observer',
                abandoned: 'Never written',
            } satisfies Record<ToolCloseReason, string>,
            formats: { ytyp: '.ytyp', modelInfo: 'model-info .meta' } satisfies Record<
                ToolFileFormat,
                string
            >,
        },
        settings: {
            title: 'Settings',
            meta: 'Saving asks for your password',
            atStart: 'At start',
            atStartModes: { off: 'Off', report: 'Report', fix: 'Fix' } satisfies Record<ToolMode, string>,
            atStartHint:
                'Off runs the tool only when you ask. Report scans before every start of FXServer and writes to the console when the result changed. Fix applies the fixes before every start. Whatever this says, every start first puts back the copies whose kept copy is gone.',
            differences: 'Differences',
            differencesModes: {
                identical: 'Identical only',
                keepLatest: 'Keep the latest copy',
            } satisfies Record<Differences, string>,
            differencesHint:
                'Keep the latest copy also fixes copies that differ, keeping the last one registered. Which copy the game uses when they differ is not known for sure: this can change the model, the textures or the draw distance players see.',
            escrowed: 'Escrowed resources',
            escrowedModes: { skip: 'Skip', allow: 'Allow' } satisfies Record<EscrowPolicy, string>,
            escrowedHint:
                'Skip only reports resources protected by the asset escrow. Allow fixes their plain files too.',
            excluded: 'Excluded resources',
            excludedHint:
                'One resource name a line, as FXServer knows it, case included. The tool never changes them: put there those a pipeline or a file group keeps.',
            excludedInvalid: 'Each name is 1 to 255 characters without / or \\, and 1,000 names at most.',
            changes: (value: number) => count(value, { one: 'unsaved change', other: 'unsaved changes' }),
            discard: 'Discard',
            save: 'Save',
            saved: 'Settings saved. The next scan, fix and start use them.',
            changedElsewhere:
                'These settings were changed in another session since you opened them. Discard your changes to see theirs, then make yours again.',
        },
    },
    server_state: {
        states: {
            stopped: 'Stopped',
            starting: 'Starting',
            online: 'Online',
            degraded: 'Degraded',
            hung: 'Not responding',
            stopping: 'Stopping',
            crashed: 'Crashed',
            crashLoop: 'Crash loop',
        } satisfies Record<ServerState, string>,
        started: (by: string) => `Started by ${by}.`,
        stopped: (by: string) => `Stopped by ${by}.`,
        stopping: (by: string) => `Stopped by ${by}: resources are told first, then FXServer quits.`,
        restarting: (by: string) =>
            `Restarting, as ${by} asked: resources are told first, then FXServer quits and starts again.`,
        restarted: (by: string) => `Restarted by ${by}.`,
        restartingOnSchedule:
            'Restarting on schedule: resources are told first, then FXServer quits and starts again.',
        restartedOnSchedule: 'Restarted on schedule.',
        restartingAsPlanned: (by: string) =>
            `Restarting, as ${by} planned: resources are told first, then FXServer quits and starts again.`,
        restartedAsPlanned: (by: string) => `Restarted as ${by} planned.`,
        observer: 'Observer',
        // Someone at the host's own status window, who signs in to nothing.
        local: 'someone at the host',
        recovery: 'Started again after a failure.',
        ready: 'Ready: its HTTP endpoint answers and the observer bridge reports in.',
        recovered: 'Healthy again.',
        unhealthy: (problems: string[]) => `Degraded: ${list(problems)}.`,
        problems: healthProblems,
        hangNoHeartbeat:
            'Not responding: the observer bridge stopped reporting in, so its main loop is stuck.',
        hangNoHttp: 'Not responding: its HTTP endpoint stopped answering.',
        hangAdvice: (minutes: number) =>
            `FXServer stayed silent for ${count(minutes, { one: 'minute', other: 'minutes' })}, the delay set under Server → Hang detection. A first start that builds its caches of clothing and vehicles can take longer: raise the delay there, or turn hang detection off for that start.`,
        openHang: 'Open hang detection',
        exited: (description: string) => `FXServer exited: ${description}.`,
        fatalError: (message: string) => `FXServer reported a fatal error: ${message}`,
        bindError: (address: string) => `${address} is already in use by another program.`,
        licenceRejected: 'The licence key was refused. FXServer stays stopped until someone starts it again.',
        consoleQuit: (by: string) => `${by} typed quit in the console.`,
        shutdown: 'Observer is stopping, and FXServer with it.',
        crashLoop: (failures: number) =>
            `${count(failures, { one: 'failure', other: 'failures' })} in a row: automatic restarts are off until someone starts FXServer.`,
        spawnFailed: (reason: SpawnFailure) => `FXServer could not start: ${spawnFailures[reason]}`,
        restartIn: (delay: string) => `Restarting in ${delay}.`,
    },
    controls: {
        label: 'Server actions',
        start: 'Start',
        stop: 'Stop',
        restart: 'Restart',
        notPermitted: 'Your account cannot start, stop or restart FXServer.',
        unknown: 'The state of FXServer is not known yet.',
        notConfigured: 'FXServer has no launch settings yet.',
        invalid: 'The launch settings need fixing on the Server page.',
        noBuild: 'No build is selected: install or import one on the Artifacts page.',
        running: 'FXServer is already running.',
        notRunning: 'FXServer is not running.',
        stopping: 'FXServer is stopping.',
        restartTitle: 'Restart FXServer?',
        stopTitle: 'Stop FXServer?',
        connected: (players: number) =>
            players === 0
                ? 'No player is connected.'
                : `${count(players, { one: 'player is', other: 'players are' })} connected. They will be disconnected.`,
        noticeFirst:
            'Resources receive observer:shuttingDown first, then FXServer quits; whatever does not end in 15 seconds is killed.',
        confirmRestart: 'Restart',
        confirmStop: 'Stop server',
        keepStoppedTitle: 'Keep FXServer stopped?',
        keepStopped:
            'FXServer is not running. Stopping it cancels any automatic restart and leaves it stopped until someone starts it.',
        confirmKeepStopped: 'Keep it stopped',
        failed: (action: string, reason: string) => `${action} failed. ${reason}`,
        unavailable: (action: string, reason: string) => `${action} is not available. ${reason}`,
        dismiss: 'Dismiss',
    },
    console: {
        title: 'Console',
        gloss: 'FXServer output, live.',
        card: 'Console',
        search: 'Search the output',
        streams: 'Streams shown',
        kinds: {
            stdout: 'stdout',
            stderr: 'stderr',
            command: 'commands',
            observer: 'observer',
        } satisfies Record<LineKind, string>,
        observerTag: 'observer',
        stderrTag: 'stderr',
        link: {
            connecting: 'connecting',
            live: 'streaming',
            offline: 'offline',
            refused: 'reload needed',
        } satisfies Record<LinkState, string>,
        paused: 'paused',
        pause: 'Pause',
        resume: 'Resume',
        download: 'Download logs',
        output: (lines: number) => `FXServer output, ${count(lines, { one: 'line', other: 'lines' })}`,
        empty: 'No line matches the search and the streams shown.',
        nothingYet: 'FXServer has printed nothing yet.',
        gap: 'Earlier lines are no longer in Observer’s memory: the log files keep them.',
        restarted:
            'Observer restarted here: whatever the previous one printed after the line above is in the log files.',
        newLines: (lines: number) => count(lines, { one: 'new line', other: 'new lines' }),
        commandLabel: 'Command to send to FXServer',
        enterKey: 'Enter',
        send: 'to send',
        history: 'history',
        stopped: 'FXServer is stopped.',
        notRunning: 'FXServer is not running.',
        stopping: 'FXServer is stopping: it takes commands until it has quit.',
        stateUnknown: 'The state of FXServer is not known: Observer answers when you send.',
        readOnly: 'Read only: your account cannot send commands.',
        sent: (command: string) => `Sent: ${command}`,
        sentMasked: 'Sent. Its secret is masked in the console and the audit log.',
        pending: 'FXServer has not read the command yet: it runs when the server reads its console again.',
        busy: 'FXServer is not reading its console, so the command was not sent. Try again in a moment.',
        summary: (lines: number, errors: number) =>
            errors === 0
                ? count(lines, { one: 'new line', other: 'new lines' })
                : `${count(lines, { one: 'new line', other: 'new lines' })}, ${count(errors, { one: 'error', other: 'errors' })}`,
        unavailable: 'The console shows again once the live link is back.',
        logs: {
            title: 'Console log files',
            body: 'A file a day, continued in a new one past 256 MB, without colours. The oldest go after 14 days, or sooner once all of them pass 2 GB. The newest is first.',
            caption: 'Console log files, newest first',
            name: 'File',
            size: 'Size',
            modified: 'Last written',
            actions: 'Actions',
            download: (name: string) => `Download ${name}`,
            none: 'No log file yet.',
            loadFailed: 'The log files could not be listed.',
        },
    },
    notFound: {
        title: 'This page does not exist.',
        back: 'Back to the panel',
    },
    forbidden: {
        title: 'Not allowed',
        gloss: 'This page is not for your account.',
        body: (permission: string) =>
            `It needs a permission your account does not hold: ${permission}. Whoever manages users can grant it.`,
        anyOf: (names: string[]) =>
            `It needs one of these permissions, which your account does not hold: ${new Intl.ListFormat('en', { type: 'disjunction' }).format(names)}. Whoever manages users can grant one.`,
        ownerOnly:
            'Only the owner opens this page: how FXServer starts decides what runs on the host, which no permission grants.',
        back: 'Open my account',
    },
    signIn: {
        home: 'Observer, home',
        preferences: 'Language and theme',
        copyright: (year: number) => `© ${year} Observer`,
        stage: {
            title: 'Your server, live',
            live: 'Live',
            marker: 'Scheduled restarts',
            fact: 'Restarts on time.',
            factTone: 'Announced first.',
            factBody: {
                before: 'Restarts follow the schedule you set, in your time zone, daylight saving included. Before each one, your resources receive ',
                code: 'observer:restartScheduled',
                after: ' at every warning step, from 30 minutes down to 1, so they can warn players and save what they must.',
            },
            tick: { name: 'svMain', value: '6.8 ms' },
            players: { name: 'players', value: '187 / 256' },
            events: 'Latest events',
            sample: 'Sample data',
            ticker: 'Latest events on Main server, sample data',
            observer: 'Observer',
            ownerInitials: 'BA',
            samples: {
                user: { title: 'New sub-user', detail: 'kenji · Moderator', figure: '+1', time: '16:09' },
                crash: {
                    title: 'Crash recovered',
                    detail: 'lua54 · dump saved',
                    figure: '12 s',
                    time: '14:04',
                },
                hitch: {
                    title: 'Hitch analysed',
                    detail: 'svMain · I/O bound',
                    figure: '412 ms',
                    time: '13:40',
                },
                build: { title: 'Build applied', detail: 'probation passed', figure: '37872', time: '06:10' },
                restart: {
                    title: 'Restart done',
                    detail: 'on schedule, warned',
                    figure: '41 s',
                    time: '06:00',
                },
            },
        },
    },
    login: {
        eyebrow: 'Sign in',
        title: 'Welcome back.',
        tone: 'Observer keeps watch, signed in or not.',
        lead: "Sign in to get back to your server's console, schedule and metrics.",
        form: 'Sign in to Observer',
        username: 'Username',
        password: 'Password',
        forgot: 'Forgot your password?',
        forgotTitle: 'Ask for a temporary password',
        forgotBody: {
            before: 'Accounts are managed by the server owner, or someone they allow to manage users. They can reset your password from the Users page, or run ',
            code: 'observer user reset-password <name>',
            after: ' on the host.',
        },
        forgotAfter: 'You then sign in with the temporary password, valid for 72 hours, and choose your own.',
        closeHelp: 'Close the help',
        remember: 'Keep me signed in',
        submit: 'Sign in',
        wait: (seconds: number) => `Try again in ${formatDuration('en', seconds)}`,
        throttled: (seconds: number) => `Too many attempts. Try again in ${formatDuration('en', seconds)}.`,
        noSignUp: 'No sign-up: the server owner, or someone they allow to manage users, creates accounts.',
        guard: 'After 5 failed attempts in 15 minutes, this username is paused from your network, for longer each time, up to 15 minutes. Failed attempts are written to the audit log.',
    },
    setup: {
        eyebrow: 'First run',
        title: 'Set up Observer.',
        tone: 'Start with the owner account.',
        lead: 'The owner holds every permission, now and later, and creates the other accounts. Choose a username and a password of at least 12 characters.',
        form: 'Create the owner account',
        username: 'Username',
        usernameHint: '3 to 32 letters, digits, dots, dashes or underscores.',
        password: 'Password',
        passwordHint: 'At least 12 characters.',
        confirm: 'Confirm the password',
        remember: 'Keep me signed in',
        submit: 'Create the owner account',
        noToken: 'This page needs the setup link Observer printed when it first started.',
        linkNote: {
            before: 'The link works once, for 30 minutes. For a new one, run ',
            code: 'observer setup-link',
            after: ' on the host.',
        },
        signInInstead: 'Go to the sign-in page',
    },
    passwordChange: {
        eyebrow: 'New password',
        title: 'Choose your own password.',
        tone: 'The temporary one ends here.',
        lead: 'Your account was given a temporary password. Choose one only you know, of at least 12 characters, to open the panel.',
        form: 'Choose a new password',
        current: 'Temporary password',
        next: 'New password',
        nextHint: 'At least 12 characters.',
        confirm: 'Confirm the new password',
        submit: 'Save and continue',
        note: 'Your other sessions end when the password changes.',
        signOut: 'Sign out instead',
    },
    reauth: {
        title: 'Confirm your password',
        body: 'This needs your password again. Once confirmed, it is not asked again for 10 minutes.',
        password: 'Password',
        submit: 'Confirm',
    },
    server: {
        title: 'Server',
        gloss: 'How Observer starts FXServer.',
        nextStart: 'Changes apply at the next start of FXServer; hang detection applies at once',
        loadFailed: 'The launch settings could not be loaded.',
        unsaved: 'Nothing is saved yet: Observer needs these settings to start FXServer.',
        runSetup: 'Run the setup',
        changedElsewhere:
            'These settings were changed in another session since you opened them. Discard your changes to see theirs, then make yours again.',
        files: 'Files',
        build: 'Build',
        buildSelected: (build: string) => `FXServer starts build ${build}, chosen on the Artifacts page.`,
        buildNone: 'No build is selected yet.',
        buildUnknown: 'The selected build is not known while the live link is down.',
        openArtifacts: 'Open the Artifacts page',
        legacyBuild:
            'FXServer ran from a folder of yours until this version. Observer now runs builds from its data folder: import it on the Artifacts page to keep using it.',
        dataPath: 'Server data folder',
        dataPathHint: 'The folder of your resources and cfg. FXServer runs from it.',
        cfgFile: 'Config file',
        cfgFileHint: 'Relative to the server data folder, such as server.cfg or configs/live.cfg.',
        start: 'Start',
        onesync: 'OneSync',
        onesyncModes: { off: 'Off', on: 'On', legacy: 'Legacy' } satisfies Record<OneSync, string>,
        onesyncHint: 'Set after your cfg on the command line, so it wins over the cfg.',
        autoStart: 'Start FXServer with Observer',
        autoStartHint: 'When Observer starts, FXServer starts too.',
        txadminEvents: "Send txAdmin's events too",
        txadminEventsHint:
            "Resources written for txAdmin also receive txAdmin's events, such as txAdmin:events:scheduledRestart, next to Observer's.",
        hang: 'Hang detection',
        hangMeta: 'Applies at once, to a running server too',
        hangDetection: 'Restart FXServer when it stops responding',
        hangDetectionHint:
            'Observer restarts FXServer once its main loop or its HTTP endpoints have stayed silent for the delay below. Off, it waits as long as they stay silent. A crash restarts FXServer either way.',
        hangTimeout: 'Restart after, in minutes',
        hangTimeoutHint:
            'From 1 to 60, 6 by default. Raise it if a first start takes longer, while resources build their caches of clothing and vehicles.',
        arguments: 'Extra arguments',
        argumentsMeta: 'Each one goes on the command line as +set <name> <value>, after the cfg.',
        argument: (position: number) => `Argument ${formatNumber('en', position)}`,
        argumentName: 'Name',
        argumentValue: 'Value',
        removeArgument: (position: number) => `Remove argument ${formatNumber('en', position)}`,
        addArgument: 'Add an argument',
        noArguments: 'No extra argument.',
        notSaved: 'Not saved yet',
        changes: (value: number) => `${count(value, { one: 'unsaved change', other: 'unsaved changes' })}`,
        discard: 'Discard',
        discarded: 'Changes discarded: the form shows the settings as stored.',
        save: 'Save',
        saved: 'Saved. FXServer uses these settings from its next start, a restart after a crash included.',
        savedHang: 'Saved. Hang detection applies at once, to a running server too.',
        savedBoth:
            'Saved. Hang detection applies at once, to a running server too; the other settings from the next start of FXServer, a restart after a crash included.',
        problems: {
            required: 'Fill this in.',
            tooLong: 'Too long: 1,024 characters at most.',
            notAbsolute: 'Use a full path, from a drive letter such as C:\\ on Windows or from / on Linux.',
            networkPath: 'Use a folder of this host: network paths are not supported.',
            devicePath: 'This names a device, not a folder or a file.',
            controlCharacter: 'Remove the line breaks, tabs and other control characters.',
            fxdk: '-fxdk is refused: it would start FXServer for the development kit, outside Observer.',
            invalidCfgFile:
                'Use a path inside the server data folder, with letters, digits, dots, dashes, underscores and / between folders.',
            optionLike:
                'Start with something other than - or +: FXServer would read it as an option or a command of its own.',
            invalidName: 'Use 1 to 64 letters, digits or underscores.',
            forbiddenCharacter:
                'Remove the quotes, semicolons and backslashes: FXServer reads them as commands.',
            quoteOrSemicolon: 'Remove the quotes and semicolons: FXServer reads them as commands.',
            reserved: 'Observer sets this one itself, or it would break supervision.',
            secret: 'This holds a secret: set it in the cfg, never on the command line, which anyone on the host can read.',
            duplicate: 'An argument above already sets this name.',
            tooMany: 'At most 64 extra arguments.',
            outOfRange: 'A whole number of minutes, from 1 to 60.',
        } satisfies Record<LaunchProblem, string>,
    },
    serverSetup: {
        title: 'Server setup',
        gloss: 'Point Observer at your server.',
        steps: 'Setup steps',
        stepNames: {
            folder: 'Server folder',
            cfg: 'Config file',
            build: 'FXServer build',
            start: 'Review and start',
        } satisfies Record<SetupStep, string>,
        stepHints: {
            folder: 'Pick one Observer found, or another',
            cfg: 'Read from the folder',
            build: 'Install one, or import yours',
            start: 'Nothing runs before this',
        } satisfies Record<SetupStep, string>,
        stepOf: (step: number, total: number, name: string) =>
            `Step ${formatNumber('en', step)} of ${formatNumber('en', total)} · ${name}`,
        said: { passed: 'done', failed: 'failed' },
        back: 'Back',
        continue: 'Continue',
        changedElsewhere:
            'The launch settings were changed in another session since the setup opened. Start again from them, then make your changes again.',
        startAgain: 'Start again from the saved settings',
        folder: {
            question: 'Which server should Observer run?',
            searching:
                'Observer is looking in the usual places on this machine: your home folder and the root of each drive.',
            found: (servers: number) =>
                servers === 0
                    ? 'Observer looked in the usual places on this machine and found no server. Point it at yours below.'
                    : `Observer looked in the usual places on this machine and found ${count(servers, { one: 'server', other: 'servers' })}. Pick one, or point it somewhere else.`,
            searchStopped:
                'The search stopped before it looked everywhere: if yours is missing, browse to it or type its path.',
            searchFailed: (reason: string) => `Observer could not search this machine. ${reason}`,
            lead: 'FXServer runs from the folder in place: Observer copies nothing into it, and writes there only what people upload over SFTP and the fixes of the server tools you run or turn on.',
            servers: 'Servers found',
            selected: 'Selected',
            resourceCount: (resources: number, truncated: boolean) =>
                `${truncated ? 'At least ' : ''}${count(resources, { one: 'resource', other: 'resources' })}`,
            onesync: {
                off: 'OneSync off',
                on: 'OneSync on',
                legacy: 'OneSync legacy',
            } satisfies Record<OneSync, string>,
            usedByTxAdmin: 'used by txAdmin',
            portInUse: (port: number) => `port ${formatNumber('en', port, { useGrouping: false })} in use`,
            portTaken: (
                port: number,
                holder: { program: string | null; fxServer: boolean },
                txAdmin: boolean,
            ) =>
                holder.fxServer
                    ? txAdmin
                        ? `FXServer already listens on port ${formatNumber('en', port, { useGrouping: false })}, and txAdmin is set up for this server. Stop it in txAdmin before the first start here, or both will fight for the port.`
                        : `Another FXServer already listens on port ${formatNumber('en', port, { useGrouping: false })}. Stop it before the first start here, or both will fight for the port.`
                    : `${holder.program ?? 'Another program'} already listens on port ${formatNumber('en', port, { useGrouping: false })}: FXServer cannot start on it until that program lets it go.`,
            elsewhere: 'Somewhere else',
            label: 'Server data folder',
            placeholder: 'Start typing a path',
            hint: 'The folder that holds your resources folder and your server.cfg. Observer lists the folders on this machine as you type.',
            browse: 'Browse…',
            rescan: 'Rescan',
            continueWith: (name: string) => `Continue with ${name}`,
            keys: { tab: 'Tab', enter: 'Enter', up: '↑', down: '↓' },
            complete: 'complete',
            move: 'move',
            chooseAndCheck: 'choose and check',
            suggestions: 'Folders',
            shown: (folders: number, truncated: boolean) =>
                truncated
                    ? `The first ${count(folders, { one: 'folder', other: 'folders' })}: keep typing to narrow them.`
                    : count(folders, { one: 'folder', other: 'folders' }),
            noMatch: 'No folder here starts with this.',
            kinds: {
                server: 'Server folder',
                txAdminData: 'txAdmin data',
                unreadable: 'Observer may not read it',
            },
            serverDetail: (resources: number | null, cfg: string | null) =>
                [resources === null ? null : count(resources, { one: 'resource', other: 'resources' }), cfg]
                    .filter((part) => part !== null)
                    .join(' · '),
            txAdminDetail: 'not a server folder',
            folderDetail: (folders: number, serverInside: boolean) =>
                `${count(folders, { one: 'folder', other: 'folders' })}${serverInside ? ' · holds a server below' : ''}`,
            link: 'link',
            browser: {
                label: 'Folders of this machine',
                typeInstead: 'Type a path instead',
                up: 'Up one folder',
                current: 'Current folder',
                filter: 'Filter folders',
                filterPlaceholder: 'Filter',
                places: 'Places',
                home: 'Home',
                found: 'Servers found',
                folders: (name: string) => `Folders in ${name}`,
                open: (name: string) => `Open ${name}`,
                holdsServer: 'holds a server',
                showSuggestion: 'Show it',
                reading: 'Reading the folder',
                empty: 'No folder here.',
                noMatch: 'No folder here matches the filter.',
                truncated: 'Only the first 200 folders are listed: type the path to reach the others.',
                selection: 'Selected folder',
                selected: 'Selected',
                checking: 'Checking the folder',
                isServer: 'This is a server folder',
                resources: 'Resources',
                inCategories: (categories: number) =>
                    `in ${count(categories, { one: 'category', other: 'categories' })}`,
                cfgCount: 'Config files',
                cfgFirst: (name: string) => `${name} first`,
                use: 'Use this folder',
            },
            problems: {
                invalidPath: 'This path cannot be used.',
                missing: 'Nothing is at this path.',
                notAFolder: 'This is a file, not a folder.',
                unreadable: 'Observer cannot list this folder: check its permissions.',
                insideObserver: "This is Observer's data folder, or inside it: use your server's folder.",
                isResources: 'This is the resources folder: use the folder that holds it.',
                noResources: 'This folder holds no resources folder.',
                emptyResources: 'Its resources folder holds no resource.',
            } satisfies Record<FolderProblem, string>,
            suggestion: 'A folder nearby holds a server:',
            useSuggestion: 'Use this folder',
            accepted: 'FXServer can run from this folder.',
            resources: (resources: number, categories: number, truncated: boolean) =>
                `${truncated ? 'At least ' : ''}${count(resources, { one: 'resource', other: 'resources' })} in ${count(categories, { one: 'category', other: 'categories' })}${truncated ? ': the count stopped at 250 categories or 5,000 files and folders' : ''}.`,
            withoutManifest: (names: string[]) =>
                `Without fxmanifest.lua or __resource.lua, FXServer does not start ${list(names)}.`,
            cfgFiles: 'Config files found',
            noCfgFile: 'No cfg was found at the root or one folder down: name yours in the next step.',
        },
        cfg: {
            lead: 'The cfg FXServer runs, and the files it runs in turn. Observer only reads them: fix what it finds in your files, then check again.',
            found: 'Found in the folder',
            detected: 'detected',
            label: 'Config file',
            hint: 'Relative to the server folder, such as server.cfg or configs/live.cfg.',
            checkAgain: 'Check again',
            checking: 'Reading the cfg',
            notChecked: 'Check the cfg to go on.',
            blocked: 'Fix what blocks in the cfg, then check again.',
            problems: {
                invalidPath: 'This path cannot be used.',
                missing: 'No file is at this path in the server folder.',
                notAFile: 'This is a folder, not a file.',
                unreadable: 'Observer cannot read this file: check its permissions.',
                tooLarge: 'This file is over 1 MB, more than a cfg ever holds.',
            } satisfies Record<CfgFileProblem, string>,
            folderRefused: 'The server folder no longer passes its check: go back to it.',
            files: 'Files',
            fileStates: {
                read: 'Read',
                missing: 'Missing',
                outside: 'Outside the folder, not checked',
                resource: 'In a resource, not checked',
                tooDeep: 'Too deep, not checked',
                repeated: 'Run again',
                tooLarge: 'Over 1 MB, not checked',
                tooMany: 'Past 32 files, not checked',
            } satisfies Record<CfgFileState, string>,
            facts: 'What the cfg sets',
            game: 'Game',
            games: { gta5: 'GTA V', rdr3: 'RedM' } satisfies Record<Game, string>,
            defaultGame: 'GTA V, as FXServer runs without gamename',
            endpoints: 'Endpoints',
            protocols: { tcp: 'TCP', udp: 'UDP' } satisfies Record<EndpointProtocol, string>,
            slots: 'Slots',
            onesync: 'OneSync',
            hostname: 'Hostname',
            licence: 'Licence key',
            licences: {
                set: 'Set',
                missing: 'Missing',
                placeholder: 'Placeholder',
                malformed: 'Not a Cfx.re key',
                unknown: 'Not found: it may be in a cfg Observer did not check',
            } satisfies Record<LicenceState, string>,
            notSet: 'Not set',
            none: 'None',
            findings: 'Findings',
            severities: { blocking: 'Blocking', warning: 'Warning', note: 'Note' } satisfies Record<
                Severity,
                string
            >,
            groups: {
                blocking: (value: number) => `Blocking · ${formatNumber('en', value)}`,
                warning: (value: number) =>
                    `${plural('en', value, { one: 'Warning', other: 'Warnings' })} · ${formatNumber('en', value)}`,
                note: (value: number) =>
                    `${plural('en', value, { one: 'Note', other: 'Notes' })} · ${formatNumber('en', value)}`,
            } satisfies Record<Severity, (value: number) => string>,
            clean: 'Nothing in the cfg works against Observer.',
            kinds: cfgFindings,
        },
        build: {
            lead: 'The FXServer build Observer starts. Install the one Cfx.re recommends, or import a folder of FXServer you already have.',
            selected: (build: string) => `FXServer starts build ${build}.`,
            none: 'No build is selected yet.',
            unknown: 'The builds are not known while the live link is down.',
            install: 'Install the recommended build',
            catalogLoading: 'Reading the builds Cfx.re lists.',
            catalogFailed:
                'The builds Cfx.re lists could not be read: import a folder instead, or try again later.',
            noRecommended: 'Cfx.re recommends no build for this system right now: import a folder instead.',
            recommendedInstalled: (build: string) =>
                `The build Cfx.re recommends, ${build}, is installed already: use it below.`,
            importFolder: 'Import a folder',
            installed: 'Installed builds',
            use: (build: string) => `Use build ${build}`,
            skip: 'Skip for now',
            openArtifacts: 'Open the Artifacts page',
        },
        start: {
            lead: 'How FXServer starts, then what it starts from.',
            summary: 'Summary',
            folder: 'Server folder',
            cfg: 'Config file',
            build: 'Build',
            noBuild: 'None yet',
            findings: 'Findings',
            findingCount: (warnings: number, notes: number) =>
                warnings + notes === 0
                    ? 'None'
                    : list(
                          [
                              warnings > 0 ? count(warnings, { one: 'warning', other: 'warnings' }) : null,
                              notes > 0 ? count(notes, { one: 'note', other: 'notes' }) : null,
                          ].filter((part) => part !== null),
                      ),
            saveAndStart: 'Save and start',
            saveAndRestart: 'Save and restart',
            saveOnly: 'Save only',
            // Its own: the setup changes nothing that reaches a running server at once.
            saved: 'Saved. FXServer uses these settings from its next start, a restart after a crash included.',
            withoutBuild:
                'No build is selected, so FXServer cannot start yet: save now, then start it once a build is selected.',
            starting: 'Saved. FXServer is starting: its first launch is followed below.',
            startFailed: (reason: string) => `Saved, but FXServer did not start. ${reason}`,
            argumentsRefused:
                'This host refuses the extra arguments saved with the launch settings, which the setup keeps as they are. Fix them on the Server page, then save here again.',
            openServer: 'Open the Server page',
        },
        launch: {
            title: 'First launch',
            steps: {
                started: 'FXServer started',
                port: 'Port open',
                licence: 'Licence accepted',
                bridge: 'Observer bridge connected',
                ready: 'Server ready',
            } satisfies Record<LaunchStep, string>,
            ready: 'FXServer is ready: players can connect.',
            unknown: 'The state of FXServer is not known while the live link is down.',
            console: "The console shows FXServer's output, and what Observer found in the cfg as it started.",
            openConsole: 'Open the console',
        },
    },
    files: {
        title: 'Files',
        gloss: 'SFTP',
        intro: 'Groups say which parts of the server folder an access reaches. People log in with their own SSH keys; pipelines with a key Observer makes. Nothing connects until the owner adds an [sftp] section to observer.toml.',
        groups: 'Groups',
        newGroup: 'New group',
        noGroups: 'No group yet. A group lists what it allows and denies, folder by folder.',
        reachTitle: 'Reach',
        rulesTitle: 'Rules',
        ruleCount: (rules: number) => (rules === 1 ? '1 rule' : `${rules} rules`),
        actions: 'Actions',
        edit: 'Edit',
        editNamed: (name: string) => `Edit ${name}`,
        delete: 'Delete',
        people: 'People',
        account: 'Account',
        keys: 'Keys',
        noAccess: 'No access',
        assignGroups: 'Groups',
        pipelines: 'Pipelines',
        newPipeline: 'New pipeline',
        noPipelines: 'No pipeline yet. A pipeline uploads from CI/CD with a key of its own.',
        keyEnds: 'Key ends',
        never: 'never',
        deleteGroup: (name: string) => `Delete ${name}?`,
        deleteGroupMessage: 'Every access holding it loses what it gives, and their sessions end.',
        deletePipeline: (name: string) => `Delete +${name}?`,
        deletePipelineMessage: 'Its key stops working at once and its sessions end.',
        reach: { read: 'Read', assets: 'Assets', code: 'Code' },
        effects: { allow: 'Allow', deny: 'Deny' },
        depths: { tree: 'This folder and everything below', folder: 'This folder only' },
        rights: {
            list: 'List',
            read: 'Read',
            create: 'Create',
            modify: 'Modify',
            rename: 'Rename',
            delete: 'Delete',
            makeFolder: 'Make folders',
            removeFolder: 'Remove folders',
        },
        group: {
            create: 'New group',
            edit: 'Edit group',
            save: 'Save',
            name: 'Name',
            description: 'Description',
            rule: (number: number) => `Rule ${number}`,
            effect: 'Effect',
            depth: 'Reaches',
            folder: 'Folder',
            folderHint:
                'Relative to the server folder, such as resources/[maps]. * and ? match within one name.',
            names: 'Names',
            namesHint: 'Optional, separated by commas: *.ytd, fxmanifest.lua. Empty means every name.',
            rights: 'Rights',
            maxSize: 'Largest file, in MiB (optional)',
            addRule: 'Add a rule',
            removeRule: 'Remove this rule',
        },
        assign: {
            title: (name: string) => `Groups of ${name}`,
            save: 'Save',
            none: 'No group yet: the owner creates them first.',
        },
        service: {
            create: 'New pipeline',
            submit: 'Create',
            name: 'Name',
            nameHint: 'Lowercase letters, digits and dashes. It logs in as +name.',
            networks: 'Allowed networks',
            networksHint: 'Optional, such as 192.0.2.0/24. Empty means any address.',
            days: 'Key life, in days (365 at most)',
            groups: 'Groups',
            keyTitle: (login: string) => `Key of ${login}`,
            keyOnce: 'This is the only time the private key is shown. Store it in your CI secrets now.',
            login: 'Login',
            privateKey: 'Private key',
            saved: 'I stored it',
            copyKey: 'Copy the key',
            copied: 'Copied',
            downloadKey: (name: string) => `Download ${name}.key`,
        },
    },
    users: {
        title: 'Users',
        gloss: 'Who signs in, and what they may do.',
        add: 'Add a user',
        caption: 'Accounts',
        user: 'User',
        permissions: 'Permissions',
        lastLogin: 'Last sign-in',
        created: 'Created',
        actions: 'Actions',
        owner: 'Owner',
        you: 'You',
        temporary: 'Temporary password',
        allPermissions: 'Every permission',
        permissionCount: (value: number) =>
            value === 0 ? 'No permission' : count(value, { one: 'permission', other: 'permissions' }),
        never: 'Never',
        actionsFor: (username: string) => `Actions for ${username}`,
        edit: 'Edit permissions',
        reset: 'Reset the password',
        revoke: 'Sign out everywhere',
        delete: 'Delete',
        loadFailed: 'The accounts could not be loaded.',
        create: {
            title: 'Add a user',
            username: 'Username',
            usernameHint: '3 to 32 letters, digits, dots, dashes or underscores.',
            submit: 'Create the account',
        },
        temporaryPassword: {
            created: (username: string) => `${username} can sign in`,
            reset: (username: string) => `New temporary password for ${username}`,
            body: 'Give them this password. It is shown once, works for 72 hours, and must be changed at the first sign-in.',
            label: 'Temporary password',
            done: 'Done',
        },
        editDialog: {
            title: (username: string) => `Permissions of ${username}`,
            submit: 'Save the permissions',
        },
        resetDialog: {
            title: (username: string) => `Reset the password of ${username}?`,
            body: 'Their sessions end now, and they sign in again with a temporary password you give them.',
            submit: 'Reset the password',
        },
        revokeDialog: {
            title: (username: string) => `Sign ${username} out everywhere?`,
            body: 'Every session of this account ends now. They can sign in again with their password.',
            submit: 'Sign out everywhere',
        },
        deleteDialog: {
            title: (username: string) => `Delete ${username}?`,
            body: 'The account and its sessions are deleted. The audit log keeps what it did. This cannot be undone.',
            confirm: (username: string) => `Type ${username} to confirm`,
            submit: 'Delete the account',
        },
        saved: (username: string) => `The permissions of ${username} were saved.`,
        revoked: (username: string) => `${username} was signed out everywhere.`,
        deleted: (username: string) => `${username} was deleted.`,
        editor: {
            legend: 'Permissions',
            implies: (names: string[]) => `Includes ${list(names)}.`,
            grantedWith: (names: string[]) => `Granted with ${list(names)}.`,
            notHeld: 'You do not hold this permission, so you cannot change it.',
            loadFailed: 'The permission catalog could not be loaded.',
        },
    },
    myAccount: {
        files: {
            title: 'File access (SFTP)',
            login: 'Login',
            noGroups: 'No file access yet: ask whoever manages accesses.',
            groups: (names: string) => `Groups: ${names}`,
            keys: 'My SSH keys',
            fingerprint: 'Key',
            lastUsed: 'Last used',
            never: 'never',
            remove: 'Remove',
            removeNamed: (fingerprint: string) => `Remove the key ${fingerprint}`,
            addKey: 'Add an SSH public key',
            addKeyHint: 'Paste one line of your .pub file. Make one with ssh-keygen -t ed25519.',
            add: 'Add key',
        },
        title: 'My account',
        gloss: 'Your password and sessions.',
        password: {
            title: 'Password',
            current: 'Current password',
            next: 'New password',
            nextHint: 'At least 12 characters.',
            confirm: 'Confirm the new password',
            submit: 'Change the password',
            changed: 'Password changed. Your other sessions were signed out.',
        },
        sessions: {
            title: 'Sessions',
            meta: 'Where your account is signed in.',
            caption: 'Your sessions',
            started: 'Started',
            lastSeen: 'Last active',
            expires: 'Expires',
            address: 'Address',
            browser: 'Browser',
            actions: 'Actions',
            current: 'This session',
            revoke: 'Sign out',
            revokeLabel: (started: string) => `Sign out the session started ${started}`,
            unknown: 'Unknown',
            revoked: 'The session was signed out.',
            loadFailed: 'The sessions could not be loaded.',
        },
    },
    audit: {
        title: 'Audit log',
        gloss: 'Every action, by whom and from where.',
        timeNote: 'Times in your time zone',
        export: 'Export CSV',
        filters: 'Filters',
        actor: 'Username',
        action: 'Action',
        anyAction: 'Any action',
        outcome: 'Outcome',
        anyOutcome: 'Any outcome',
        from: 'From',
        to: 'Until',
        apply: 'Apply',
        clear: 'Clear',
        caption: 'Audit entries, newest first',
        time: 'Time',
        actorColumn: 'Actor',
        target: 'Target',
        address: 'Address',
        details: 'Details',
        empty: 'No entry matches these filters.',
        loadFailed: 'The audit log could not be loaded.',
        loadMore: 'Load more',
        shown: (value: number) => `${count(value, { one: 'entry', other: 'entries' })} shown`,
        allShown: (value: number) => `All ${count(value, { one: 'entry', other: 'entries' })} shown`,
        actions: {
            setup_completed: 'Setup completed',
            setup_link_created: 'Setup link created',
            login_succeeded: 'Signed in',
            login_failed: 'Sign-in failed',
            logout: 'Signed out',
            session_revoked: 'Session ended',
            password_changed: 'Password changed',
            user_created: 'User created',
            user_deleted: 'User deleted',
            user_permissions_changed: 'Permissions changed',
            user_password_reset: 'Password reset',
            user_unlocked: 'User unlocked',
            permission_denied: 'Permission denied',
            audit_exported: 'Audit log exported',
            reauthenticated: 'Password confirmed',
            launch_settings_changed: 'Launch settings changed',
            server_started: 'Server started',
            server_stopped: 'Server stopped',
            server_restarted: 'Server restarted',
            console_command_sent: 'Console command',
            console_log_downloaded: 'Console log downloaded',
            server_crash_loop: 'Crash loop: restarts stopped',
            schedule_changed: 'Schedule changed',
            scheduled_restart_skipped: 'Scheduled restart skipped',
            scheduled_restart_restored: 'Scheduled restart restored',
            scheduled_restart_delayed: 'Scheduled restart delayed',
            scheduled_restart_missed: 'Scheduled restart missed',
            one_off_restart_planned: 'Single restart planned',
            one_off_restart_cancelled: 'Single restart cancelled',
            build_install_started: 'Build install started',
            build_install_cancelled: 'Build install cancelled',
            build_install_finished: 'Build install finished',
            build_selected: 'Build selected',
            pending_build_set: 'Build set for the scheduled restart',
            pending_build_cancelled: 'Pending build cancelled',
            pending_build_applied: 'Pending build applied',
            pending_build_dropped: 'Pending build not applied',
            probation_passed: 'Probation passed',
            probation_failed: 'Probation failed',
            build_rolled_back: 'Went back to the last healthy build',
            build_deleted: 'Build deleted',
            artifact_settings_changed: 'Artifact settings changed',
            observer_stopped: 'Observer stopped',
            tree_process_started: 'Process started by FXServer',
            hang_snapshot_taken: 'Hung server pictured',
            crash_file_downloaded: 'Crash file downloaded',
            crash_files_deleted: 'Crash files deleted',
            tree_records_full: 'Process records full',
            file_session_opened: 'File session opened',
            file_session_closed: 'File session closed',
            file_login_failed: 'File sign-in failed',
            file_written: 'File written',
            file_deleted: 'File deleted',
            folder_created: 'Folder created',
            folder_removed: 'Folder removed',
            file_renamed: 'File renamed',
            file_refused: 'File operation refused',
            file_records_full: 'File records full',
            file_group_created: 'File group created',
            file_group_changed: 'File group changed',
            file_group_deleted: 'File group deleted',
            file_access_changed: 'File access changed',
            service_access_created: 'Pipeline created',
            service_access_changed: 'Pipeline changed',
            service_access_deleted: 'Pipeline deleted',
            file_key_added: 'SFTP key added',
            file_key_removed: 'SFTP key removed',
            file_session_disconnected: 'File session ended',
            file_settings_changed: 'File settings changed',
            file_host_key_created: 'SFTP host key created',
            player_forgotten: 'Player forgotten',
            players_imported: "txAdmin's players imported",
            players_retention_changed: 'Player retention changed',
            player_drops_exported: 'Drop log exported',
            drop_rates_exported: 'Drop rates exported',
            drops_retention_changed: 'Drop retention changed',
            change_contents_viewed: 'Changed file read',
            change_settings_changed: 'Change tracking settings changed',
            change_contents_purged: 'Kept contents of a file purged',
            change_verify_requested: 'Full verification of the files asked for',
            change_check_requested: 'Check of the disk asked for',
            tool_settings_changed: 'Tool settings changed',
            tool_files_changed: 'Files changed by a tool',
            tool_files_restored: 'Files restored by a tool',
            feature_changed: 'Feature turned on or off',
        } satisfies Record<AuditAction, string>,
        outcomes: {
            success: 'Success',
            failure: 'Failure',
            denied: 'Denied',
        } satisfies Record<Outcome, string>,
        actors: {
            user: 'User',
            local: 'Host window',
            cli: 'Command line',
            system: 'Observer',
            anonymous: 'Anonymous',
            service: 'Pipeline',
        } satisfies Record<ActorKind, string>,
        // A failed sign-in whose username names no account: it may have been a password.
        unknownUsername: 'Unknown username',
        noPermission: 'none',
        unset: 'not set',
        // A schedule with no zone of its own follows the host's.
        hostZone: "the host's zone",
        minutes: (value: string) => `${value} min`,
        rate: (value: string) => `${value}/s`,
        // Change tracking's sizes, in the units its settings are typed in.
        megabytes: (value: string) => `${value} MB`,
        gigabytes: (value: string) => `${value} GB`,
        // How a read of changed contents showed them.
        views: { diff: 'the diff', file: 'the whole file' },
        // What a change of the schedule's settings took away with the occurrence it removed.
        dropped: {
            skip: 'a skip',
            delay: 'a delay',
        },
        buildKinds: { install: 'Install', import: 'Import' } satisfies Record<JobKind, string>,
        applyModes: {
            now: 'Restart now',
            nextStart: 'At the next start',
            scheduledRestart: 'At the next scheduled restart',
        } satisfies Record<ApplyMode, string>,
        // Why a probation failed or a build was kept, and the failure codes of a job.
        buildReasons: {
            crashed: 'crashed',
            hung: 'stopped answering',
            disabled: 'automatic return off',
            noHealthyBuild: 'no healthy build',
            broken: 'broken',
            kvp: 'would cross build 6449',
            notInstalled: 'deleted',
            replaced: 'another build chosen since',
            unreachable: 'Cfx.re unreachable',
            httpStatus: 'HTTP error',
            archiveTooLarge: 'archive too large',
            badLength: 'wrong length',
            truncated: 'cut short',
            archiveChanged: 'archive changed',
            diskSpace: 'disk full',
            unsafeEntry: 'unsafe entry',
            damaged: 'damaged archive',
            layout: 'incomplete build',
            signature: 'not signed by Rockstar Games',
            alreadyInstalled: 'installed already',
            cancelled: 'cancelled',
            interrupted: 'Observer stopped',
            io: 'file error',
            notAFolder: 'not a folder',
            link: 'link',
            insideArtifacts: 'inside the artifacts folder',
            tooManyFiles: 'too many files',
            tooLarge: 'too large',
        },
        // A launch setting only the owner may read: the log says it changed, never what it holds.
        changed: 'changed',
        argumentsSet: (names: string) => `set ${names}`,
        argumentsRemoved: (names: string) => `removed ${names}`,
        argumentsReordered: 'reordered',
        seconds: (value: string) => `${value} s`,
        yes: 'yes',
        no: 'no',
        detail: {
            userId: 'Account id',
            sessionId: 'Session id',
            sessions: 'Sessions ended',
            permissions: 'Permissions',
            added: 'Added',
            removed: 'Removed',
            permission: 'Permission',
            username: 'Username',
            network: 'Network',
            failures: 'Failures',
            firstAt: 'First',
            lastAt: 'Last',
            lockedNetworks: 'Networks locked out',
            repeated: 'Repeated',
            attempted: 'Attempted',
            refusal: 'Reason',
            actor: 'Username',
            action: 'Action',
            outcome: 'Outcome',
            from: 'From',
            to: 'Until',
            ownerOnly: 'Owner only',
            buildPath: 'Build folder',
            build: 'Build',
            fromBuild: 'From build',
            toBuild: 'To build',
            previous: 'Replaced',
            step: 'Step',
            apply: 'Applied',
            why: 'Kept because',
            bytes: 'Size',
            archiveSha256: 'Archive SHA-256',
            allowEndOfLife: 'Past its end of life',
            endOfLifeAllowed: 'Past its end of life',
            kvpConfirmed: 'Below 6449 confirmed',
            adopted: 'Adopted from the launch settings',
            first: 'First build installed',
            cleanup: 'Cleanup',
            rollback: 'Back to the last healthy build',
            keepBuilds: 'Builds kept',
            autoRollback: 'Automatic return',
            branches: 'Test branches',
            dataPath: 'Data folder',
            cfgFile: 'Config file',
            onesync: 'OneSync',
            autoStart: 'Start with Observer',
            txadminEvents: 'txAdmin events',
            hangDetection: 'Hang detection',
            hangTimeoutSeconds: 'Restart a hung server after',
            extraArguments: 'Extra arguments',
            setup: 'From the server setup',
            command: 'Command',
            automatic: 'With Observer',
            reason: 'Reason',
            at: 'At',
            occurrence: 'Scheduled for',
            previousAt: 'Before',
            startedAt: 'Started',
            source: 'From',
            plannedBy: 'Planned by',
            lateBySeconds: 'Late by',
            state: 'State',
            scheduled: 'On schedule',
            dropped: 'Dropped',
            enabled: 'On',
            mode: 'Mode',
            times: 'Times',
            intervalMinutes: 'Interval',
            timeZone: 'Time zone',
            warnings: 'Warnings',
            skipAfterStart: 'Skip after a start',
            kind: 'Kind',
            by: 'By',
            name: 'Program',
            image: 'Image',
            commandLine: 'Command line',
            pid: 'Pid',
            parent: 'Parent',
            generation: 'Run',
            file: 'File',
            tookMs: 'Took',
            crash: 'Crash',
            names: 'Files',
            unrecorded: 'Command lines not kept',
            // A read of changed contents compares two snapshots: starts, or a start and a check of the disk.
            fromSnapshot: 'From snapshot',
            toSnapshot: 'To snapshot',
            fromPath: 'Path before',
            resource: 'Resource',
            view: 'Read',
            versions: 'Versions cleared',
            extensions: 'Extensions with line diffs',
            excluded: 'Never walked',
            hidden: 'Contents never kept',
            maxText: 'Largest text kept',
            budget: 'Contents kept at most',
            retentionDays: 'Starts kept',
            hashRate: 'Hashing rate',
            verifyDays: 'Full verification every',
            startEvent: 'observer:startChanges',
            before: 'Before',
            after: 'After',
            linkDays: 'Identifiers and names',
            addressDays: 'Address fingerprints',
            dayDays: 'Play time per day',
            profileDays: 'Profiles',
            read: 'Read',
            written: 'Written',
            imported: 'New',
            updated: 'Updated',
            forgotten: 'Skipped as forgotten',
            invalid: 'Invalid',
        },
        days: (value: number) => count(value, { one: 'day', other: 'days' }),
        // A profile whose retention is unset is never deleted.
        keptForGood: 'kept for good',
        snapshotKinds: { dump: 'a stack dump', report: 'a thread report' },
        snapshotFailures: {
            lowDisk: 'less than 1 GB free',
            timeout: 'it took over 15 s',
            failed: 'the system refused it',
        },
        milliseconds: (value: string) => `${value} ms`,
        sources: {
            schedule: 'the schedule',
            oneOff: 'a single restart',
        },
        // Where the request to stop Observer came from.
        stopSources: {
            window: 'the host window',
            console: "Observer's console",
            signal: 'a signal',
        } satisfies Record<StopSource, string>,
        missReasons: {
            late: 'Observer saw the time pass too late',
            serverBusy: 'FXServer was busy',
            launchInvalid: 'Launch settings this host refuses',
        },
        dropReasons: {
            serverRestarted: 'FXServer restarted first',
            serverStopped: 'FXServer stopped first',
            recentStart: 'FXServer started shortly before',
        },
        launchFailures: {
            notConfigured: 'No launch settings',
            invalidSettings: 'Settings this host refuses',
            notABuild: 'Not an FXServer build',
            missingData: 'No server data folder',
            cfgUnreadable: 'Unreadable server cfg',
            bridge: 'Bridge not written',
            spawn: 'Refused by the system',
        } satisfies Record<SpawnFailure, string>,
        refusals: {
            reserved: 'Reserved to Observer',
            escalation: 'Permissions they do not hold',
            owner_protected: 'The owner account',
            hierarchy: 'An account holding more than they do',
            self_edit: 'Their own account',
        },
    },
    about: {
        title: 'About',
        gloss: 'This build of Observer, and the software it is made with.',
        build: 'This build',
        version: 'Version',
        commit: 'Commit',
        development: 'Development build',
        commitDate: 'Commit date',
        platform: 'Platform',
        profile: 'Profile',
        profiles: { release: 'Release', debug: 'Debug' },
        systems: { windows: 'Windows', linux: 'Linux' },
        thirdParty: 'Third-party software',
        counts: (crates: number, npm: number) =>
            `${count(crates, { one: 'crate', other: 'crates' })} · ${count(npm, { one: 'npm package', other: 'npm packages' })}`,
        search: 'Search by name or licence',
        ecosystems: { crate: 'Crates', npm: 'npm packages' },
        name: 'Name',
        licence: 'Licence',
        noLicence: 'None declared',
        licenceText: 'Licence text',
        textOf: (name: string, version: string) => `Licence text of ${name} ${version}`,
        repository: 'Repository',
        fromSpdx: 'Text from the SPDX licence list: the package ships none.',
        noMatch: 'No package matches the search.',
        missing: 'This build carries no licence list. cargo xtask web and cargo xtask dist write it.',
        loadFailed: 'The licence list could not be loaded.',
    },
    permissions,
    permissionAreas: {
        dashboard: 'Dashboard',
        console: 'Console',
        server: 'Server',
        schedule: 'Schedule',
        artifacts: 'Builds',
        users: 'Users',
        audit: 'Audit log',
        settings: 'Settings',
        logs: 'Logs',
        diagnostics: 'Diagnostics',
        files: 'Files',
        players: 'Players',
        drops: 'Player drops',
        changes: 'Changes',
    } satisfies Record<PermissionArea, string>,
    danger: {
        sensitive: 'Sensitive',
        critical: 'Critical',
    },
    validation: {
        required: 'Fill this in.',
        passwords_differ: 'The two passwords differ.',
    },
    errors: {
        unauthenticated: 'Your session has ended. Sign in again.',
        invalid_credentials: 'Wrong username or password.',
        temporary_password_expired:
            'This temporary password has expired. Ask whoever manages users for a new one.',
        too_many_attempts: (params) =>
            `Too many attempts. Try again in ${formatDuration('en', numberParam(params, 'retryAfter'))}.`,
        busy: (params) =>
            `Observer is busy checking other passwords. Try again in ${formatDuration('en', numberParam(params, 'retryAfter'))}.`,
        too_many_sockets:
            'Too many tabs of the panel are open in this browser. Close a few, then reload this one.',
        password_change_required: 'Choose a new password before you go on.',
        reauth_required: 'This needs your password again.',
        wrong_password: 'Wrong password.',
        forbidden: (params) => {
            const key = stringParam(params, 'permission');
            return isPermissionKey(key)
                ? `Your account does not hold the permission this needs: ${permissions[key].name}.`
                : 'Your account does not hold the permission this needs.';
        },
        owner_only: 'Only the owner can do this.',
        self_edit: 'Your own account changes from My account, not from here.',
        owner_protected: 'Nobody else can change the owner account.',
        escalation: (params) =>
            `You can only grant permissions you hold yourself, not ${permissionNames(stringParam(params, 'permissions'))}.`,
        hierarchy: 'You can only do this to accounts whose every permission you hold.',
        conflict: 'This account changed since you opened it. Check it again, then retry.',
        not_found: 'This no longer exists. Someone may have deleted it.',
        name_taken: 'This username is taken.',
        invalid_username: 'Use 3 to 32 letters, digits, dots, dashes or underscores.',
        password_too_short: (params) =>
            `Use at least ${formatNumber('en', numberParam(params, 'min'))} characters.`,
        password_too_long: (params) =>
            `This password is too long: ${formatNumber('en', numberParam(params, 'max'))} bytes at most.`,
        password_unchanged: 'Choose a password different from the current one.',
        unknown_permissions: 'This Observer does not know some of these permissions. Reload the page.',
        invalid_launch_settings: 'Some settings need fixing. Each one says why below its field.',
        launch_not_configured:
            'FXServer has no launch settings yet. The owner saves them in the server setup.',
        launch_failed: (params) => {
            const reason = stringParam(params, 'reason');
            return isSpawnFailure(reason)
                ? `FXServer could not start: ${spawnFailures[reason]}`
                : 'FXServer could not start. The console says why.';
        },
        server_running: 'FXServer is already running.',
        server_not_running: 'FXServer is not running.',
        server_stopping: 'FXServer is stopping. Start it again once it has stopped.',
        shutting_down: 'Observer is stopping, and FXServer with it.',
        command_refused: (params) => {
            const reason = stringParam(params, 'reason');
            return isCommandRefusal(reason) ? commandRefusals[reason] : 'FXServer was not sent this command.';
        },
        too_many_commands: (params) =>
            `Too many commands. Send the next one in ${formatDuration('en', numberParam(params, 'retryAfter'))}.`,
        already_set_up: 'Observer is already set up. Sign in instead.',
        invalid_setup_token:
            'This setup link has expired or was already used. Run observer setup-link on the host for a new one.',
        origin_mismatch: 'Observer refused a request from another address. Reload the page.',
        unknown_host:
            'Observer does not answer to this name. Open it by its address, or add the name to http.hosts in observer.toml.',
        json_required: 'Observer refused a malformed request. Reload the page.',
        invalid_request: 'Observer refused a malformed request. Reload the page.',
        internal: 'Observer hit an internal error. Its log has the details.',
        invalid_schedule: 'Some settings need fixing. Each one says why next to its field.',
        nothing_to_skip: 'The schedule has no restart ahead to skip.',
        already_skipped:
            'A restart of the schedule is skipped already: restore it first, one can be skipped at a time.',
        nothing_to_restore: 'No skipped restart is still ahead.',
        nothing_to_delay:
            'Only a restart of the schedule can be delayed, and the next one is a single restart.',
        delay_too_long: (params) => {
            const minutes = numberParam(params, 'maxMinutes');
            return minutes > 0
                ? `It can move ${formatDuration('en', minutes * 60)} later at most: no more than 12 hours past its time, and before the next one.`
                : 'It cannot move any later: no more than 12 hours past its time, and before the next one.';
        },
        no_one_off: 'No single restart is planned.',
        job_running: 'An install or an import is running already. Wait for it to end, or cancel it.',
        no_job: 'No install or import is running.',
        job_committing: 'The build is being installed: it can no longer be cancelled.',
        catalog_unavailable:
            'Cfx.re did not answer, and Observer has no list of builds from the last day. Try again later.',
        build_not_listed: 'Cfx.re does not list this build, even after a refresh.',
        not_downloadable: (params) =>
            stringParam(params, 'reason') === 'noArchive'
                ? 'Cfx.re has no archive of this build for this system.'
                : 'This build cannot be downloaded: only the support policy names it.',
        already_installed: 'This build is installed already.',
        end_of_life:
            'This build is past its end of life: players cannot join a server running it. Allow it to go on anyway.',
        build_not_installed: 'This build is not installed. Reload the page.',
        build_broken: (params) =>
            params.reason === 'archiveChanged'
                ? 'This build came from an archive other than the one first installed for it, so it is not trusted.'
                : 'This build is broken: delete it, then install it again.',
        kvp_rollback: (params) =>
            `Build ${formatNumber('en', numberParam(params, 'highestRun'), { useGrouping: false })} ran here: going below build 6449 may delete the server's KVP data. Type 6449 to confirm.`,
        no_scheduled_restart: 'No scheduled restart is ahead: apply the build at the next start instead.',
        already_selected: 'This build starts next already.',
        no_pending: 'No build waits for the next scheduled restart.',
        build_protected: 'This build is in use: it cannot be deleted.',
        build_in_use: 'A process still runs from this build. Stop it, then delete the build.',
        invalid_artifact_settings: 'Some settings need fixing. Each one says why next to its field.',
        invalid_import_folder: 'This folder cannot hold a build FXServer runs: the field says why.',
        no_build_selected: 'No build is selected yet. Install or import one on the Artifacts page.',
        crash_not_found: 'This crash is no longer recorded.',
        crash_file_not_found: 'This file is no longer on the host.',
        crash_file_too_large: (params) =>
            `This file is over ${formatBytes('en', numberParam(params, 'maxBytes'))}: it stays on the host, in Observer's data folder.`,
        setup_blocked: (params) => {
            if (typeof params.folder === 'string') {
                return 'The server folder no longer passes its check, so nothing was saved: the check below says why.';
            }
            if (typeof params.cfg === 'string') {
                return 'The cfg can no longer be read, so nothing was saved: the check below says why.';
            }
            return Array.isArray(params.findings)
                ? 'The cfg now keeps FXServer from starting under Observer, so nothing was saved: the check below says what blocks.'
                : 'The server folder or its cfg keeps FXServer from starting under Observer. Check them again, fix what blocks, then save.';
        },
        invalid_rule: (params) =>
            typeof params.index === 'number'
                ? `Rule ${params.index + 1} needs fixing: check its ${String(params.field ?? 'fields')}.`
                : 'A rule needs fixing.',
        invalid_public_key: (params) => {
            if (params.reason === 'certificate') {
                return 'Certificates are not accepted: paste the public key itself.';
            }
            if (params.reason === 'algorithm') {
                return 'Only Ed25519 and ECDSA (P-256, P-384) keys are accepted. Make one with ssh-keygen -t ed25519.';
            }
            return 'This is not an OpenSSH public key line. Paste the whole line of the .pub file.';
        },
        key_in_use: 'This key is already registered.',
        invalid_access: (params) =>
            params.field === 'name'
                ? 'A pipeline name is 1 to 32 lowercase letters, digits and dashes.'
                : params.field === 'networks'
                  ? 'Each network is an address or a block such as 192.0.2.0/24.'
                  : 'A pipeline key lasts 1 to 365 days.',
        too_many_keys: (params) =>
            typeof params.max === 'number' ? `An access holds ${params.max} keys at most.` : 'Too many keys.',
        player_not_found: 'This player is no longer recorded: they were forgotten, or their profile expired.',
        address_search: 'A search never takes an address. Search by a name, a licence or another identifier.',
        invalid_search:
            'What follows this prefix is not a valid identifier of its kind. Check it, or search by a name.',
        invalid_players_settings:
            'Some retentions are out of range. Each one says its range below its field.',
        invalid_players_file: (params) => {
            const problem = stringParam(params, 'problem');
            return isPlayersFileProblem(problem)
                ? playersFileProblems[problem]
                : "This file cannot be imported as txAdmin's players database.";
        },
        invalid_drops_settings: (params) =>
            typeof params.min === 'number' && typeof params.max === 'number'
                ? `Keep sessions between ${params.min} and ${params.max} days.`
                : 'This retention is out of range.',
        changes_unavailable: 'The history of changes cannot be read right now. Try again in a moment.',
        start_not_found: 'This start is no longer in the history.',
        content_hidden: 'The contents of this file are never shown.',
        content_not_kept: 'The contents of this version are no longer kept.',
        diff_too_large: (params) =>
            typeof params.lines === 'number'
                ? `This file changed in ${count(params.lines, { one: 'line', other: 'lines' })}, more than a diff shows.`
                : 'This file changed in more lines than a diff shows.',
        check_running: 'A check of the files is already running. Wait for it to end.',
        check_too_soon: (params) =>
            `The files were checked moments ago. Try again in ${formatDuration('en', numberParam(params, 'retryAfter'))}.`,
        invalid_glob: (params) =>
            typeof params.index === 'number'
                ? `Pattern ${params.index + 1} cannot be used: use * and ? as wildcards, / between folders, and no .. or drive letter.`
                : 'A pattern cannot be used.',
        plan_changed:
            'The duplicates changed since this report, so nothing was applied. Check the new report, then fix again.',
        server_not_stopped: 'Stop FXServer first: the tools change files only while it is stopped.',
        tool_not_available: 'This tool works on GTA V servers only: this server runs RedM.',
        feature_disabled: (params) => {
            const key = stringParam(params, 'feature');
            const name = isFeature(key) ? features[key] : 'This part of Observer';
            switch (stringParam(params, 'held')) {
                case 'host':
                    return `${name} is turned off in observer.toml on the host.`;
                case 'notConfigured':
                    return `${name} is not set up: it needs its address in observer.toml on the host.`;
                case 'requires':
                    return `${name} needs another part of Observer that does not run.`;
                default:
                    return `${name} does not run on this Observer.`;
            }
        },
        feature_locked: (params) => {
            const key = stringParam(params, 'feature');
            const name = isFeature(key) ? features[key] : 'This part of Observer';
            return `${name} is turned off in observer.toml on the host: only a change there turns it back on.`;
        },
    } satisfies Record<ErrorCode, ErrorMessage>,
    failure: {
        unknown: 'Something went wrong. Try again.',
        network: 'Observer cannot be reached. Check that it is running, then try again.',
        code: 'This page could not be loaded, and stays so until the panel is loaded again. Reload it once Observer answers.',
    },
};

export type Messages = typeof en;
