// The source catalog. Every other locale is typed against it, so a key missing there fails the type
// check. Messages that take values are functions; they format with the helpers bound to this locale.

import type { ActorKind } from '../api/generated/ActorKind';
import type { AuditAction } from '../api/generated/AuditAction';
import type { ErrorCode } from '../api/generated/ErrorCode';
import type { Outcome } from '../api/generated/Outcome';
import type { PermissionArea } from '../api/generated/PermissionArea';
import { parsePermissions, permissionKeys, type PermissionKey } from '../auth/permissions';
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
        administration: 'Administration',
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
        observer: 'Observer',
        version: 'Version',
        ok: 'Running',
        down: 'Unreachable',
        pending: 'Checking',
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
        gloss: 'Observer, right now.',
        stateTitle: 'State',
        running: 'Observer is running.',
        version: (version: string) => `Version ${version}`,
        unreachable: 'Observer cannot be reached.',
        unreachableHint: 'The panel keeps trying every few seconds.',
        loading: 'Checking Observer',
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
        password_change_required: 'Choose a new password before you go on.',
        reauth_required: 'This change needs your password again.',
        wrong_password: 'Wrong password.',
        forbidden: (params) => {
            const key = stringParam(params, 'permission');
            return isPermissionKey(key)
                ? `Your account does not hold the permission this needs: ${permissions[key].name}.`
                : 'Your account does not hold the permission this needs.';
        },
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
        already_set_up: 'Observer is already set up. Sign in instead.',
        invalid_setup_token:
            'This setup link has expired or was already used. Run observer setup-link on the host for a new one.',
        origin_mismatch: 'Observer refused a request from another address. Reload the page.',
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
