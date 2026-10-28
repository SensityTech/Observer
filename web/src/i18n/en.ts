// The source catalog. Every other locale is typed against it, so a key missing there fails the type
// check. Messages that take values are functions; they format with the helpers bound to this locale.

import type { ActorKind } from '../api/generated/ActorKind';
import type { AuditAction } from '../api/generated/AuditAction';
import type { CommandRefusal } from '../api/generated/CommandRefusal';
import type { ErrorCode } from '../api/generated/ErrorCode';
import type { HealthProblem } from '../api/generated/HealthProblem';
import type { LaunchProblem } from '../api/generated/LaunchProblem';
import type { LineKind } from '../api/generated/LineKind';
import type { OneSync } from '../api/generated/OneSync';
import type { Outcome } from '../api/generated/Outcome';
import type { PermissionArea } from '../api/generated/PermissionArea';
import type { ServerState } from '../api/generated/ServerState';
import type { SpawnFailure } from '../api/generated/SpawnFailure';
import { parsePermissions, permissionKeys, type PermissionKey } from '../auth/permissions';
import type { LinkState } from '../live/realtime';
import { formatDuration, formatNumber, plural } from './format';
import { numberParam, stringParam } from './params';

type Params = Record<string, unknown>;
export type ErrorMessage = string | ((params: Params) => string);

const count = (value: number, forms: { one: string; other: string }) =>
    `${formatNumber('en', value)} ${plural('en', value, forms)}`;

const list = (items: string[]) => new Intl.ListFormat('en', { type: 'conjunction' }).format(items);

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
    serverControl: { name: 'Start and stop the server', description: 'Start, stop and restart FXServer.' },
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
} satisfies Record<PermissionKey, { name: string; description: string }>;

const permissionNames = (hex: string) =>
    list(permissionKeys(parsePermissions(hex)).map((key) => permissions[key].name));

const isPermissionKey = (key: string): key is PermissionKey => Object.hasOwn(permissions, key);

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

export const en = {
    app: {
        name: 'Observer',
        skipToContent: 'Skip to content',
    },
    common: {
        cancel: 'Cancel',
        close: 'Close',
        copy: 'Copy',
        copied: 'Copied',
        copyBlocked: 'Copying is blocked here: the text is selected, copy it with your keyboard.',
        showPassword: 'Show the password',
        loading: 'Loading',
        retry: 'Try again',
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
        server: 'Server',
        users: 'Users',
        audit: 'Audit log',
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
    },
    overview: {
        title: 'Overview',
        gloss: 'FXServer and Observer, right now.',
        stateTitle: 'FXServer',
        observerTitle: 'Observer',
        running: 'Observer is running.',
        version: (version: string) => `Version ${version}`,
        started: (time: string, uptime: string) => `Started ${time} · up ${uptime}`,
        unreachable: 'Observer cannot be reached.',
        unreachableHint: 'The panel keeps trying every few seconds.',
        loading: 'Checking Observer',
        serverLoading: 'Reading the state of FXServer',
        serverUnknown: 'The state of FXServer is not known while the live link is down.',
        serverForbidden: 'Your account cannot see the state of FXServer.',
        build: (build: string) => `Build ${build}`,
        players: (players: string) => `${players} players`,
        notConfigured: 'FXServer has no launch settings yet.',
        notConfiguredOwner: 'Set the build and data folders on the Server page to start it.',
        notConfiguredOther: 'The owner sets them on the Server page.',
        invalid: 'This host refuses the stored launch settings.',
        invalidOwner: 'Fix them on the Server page before the next start.',
        invalidOther: 'The owner fixes them on the Server page.',
        openServer: 'Open the Server page',
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
        observer: 'Observer',
        recovery: 'Started again after a failure.',
        ready: 'Ready: its HTTP endpoint answers and the observer bridge reports in.',
        recovered: 'Healthy again.',
        unhealthy: (problems: string[]) => `Degraded: ${list(problems)}.`,
        problems: healthProblems,
        hangNoHeartbeat:
            'Not responding: the observer bridge stopped reporting in, so its main loop is stuck.',
        hangNoHttp: 'Not responding: its HTTP endpoint stopped answering.',
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
        failed: (action: string, reason: string) => `${action} failed. ${reason}`,
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
        streaming: 'streaming',
        paused: 'paused',
        pause: 'Pause',
        resume: 'Resume',
        download: 'Download logs',
        output: (lines: number) => `FXServer output, ${count(lines, { one: 'line', other: 'lines' })}`,
        empty: 'No line matches the search and the streams shown.',
        nothingYet: 'FXServer has printed nothing yet.',
        gap: 'Earlier lines are no longer in Observer’s memory: the log files keep them.',
        newLines: (lines: number) => count(lines, { one: 'new line', other: 'new lines' }),
        commandLabel: 'Command to send to FXServer',
        send: 'to send',
        history: 'history',
        stopped: 'FXServer is stopped.',
        readOnly: 'Read only: your account cannot send commands.',
        sent: (command: string) => `Sent: ${command}`,
        pending: 'FXServer has not read the command yet: it runs when the server reads its console again.',
        busy: 'FXServer is not reading its console, so the command was not sent. Try again in a moment.',
        summary: (lines: number, errors: number) =>
            errors === 0
                ? count(lines, { one: 'new line', other: 'new lines' })
                : `${count(lines, { one: 'new line', other: 'new lines' })}, ${count(errors, { one: 'error', other: 'errors' })}`,
        unavailable: 'The console shows again once the live link is back.',
        logs: {
            title: 'Console log files',
            body: 'One file per day, without colours, kept 14 days. The newest is first.',
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
        body: 'This change needs your password again. Once confirmed, it is not asked again for 10 minutes.',
        password: 'Password',
        submit: 'Confirm',
    },
    server: {
        title: 'Server',
        gloss: 'How Observer starts FXServer.',
        nextStart: 'Changes apply at the next start of FXServer',
        loadFailed: 'The launch settings could not be loaded.',
        unsaved: 'Nothing is saved yet: Observer needs these settings to start FXServer.',
        changedElsewhere:
            'These settings were changed in another session since you opened them. Discard your changes to see theirs, then make yours again.',
        files: 'Files',
        buildPath: 'FXServer build folder',
        buildPathHint: 'The folder of FXServer.exe on Windows, or the one holding alpine on Linux.',
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
        } satisfies Record<LaunchProblem, string>,
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
        } satisfies Record<ActorKind, string>,
        // A failed sign-in whose username names no account: it may have been a password.
        unknownUsername: 'Unknown username',
        noPermission: 'none',
        unset: 'not set',
        // A launch setting only the owner may read: the log says it changed, never what it holds.
        changed: 'changed',
        argumentsSet: (names: string) => `set ${names}`,
        argumentsRemoved: (names: string) => `removed ${names}`,
        argumentsReordered: 'reordered',
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
            dataPath: 'Data folder',
            cfgFile: 'Config file',
            onesync: 'OneSync',
            autoStart: 'Start with Observer',
            txadminEvents: 'txAdmin events',
            extraArguments: 'Extra arguments',
            command: 'Command',
            automatic: 'With Observer',
        },
        refusals: {
            reserved: 'Reserved to Observer',
            escalation: 'Permissions they do not hold',
            owner_protected: 'The owner account',
            hierarchy: 'An account holding more than they do',
            self_edit: 'Their own account',
        },
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
        reauth_required: 'This change needs your password again.',
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
            'FXServer has no launch settings yet. The owner saves them on the Server page.',
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
    } satisfies Record<ErrorCode, ErrorMessage>,
    failure: {
        unknown: 'Something went wrong. Try again.',
        network: 'Observer cannot be reached. Check that it is running, then try again.',
    },
};

export type Messages = typeof en;
