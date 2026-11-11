import {
    Ban,
    Blocks,
    CalendarClock,
    ChartLine,
    Contact,
    DatabaseBackup,
    DatabaseZap,
    FileCog,
    GitCompare,
    Globe,
    LayoutDashboard,
    Map as MapIcon,
    Package,
    Radar,
    ScrollText,
    Settings,
    SquareTerminal,
    Stethoscope,
    Unplug,
    Wrench,
    type LucideIcon,
} from 'lucide-react';

import type { Me } from '../api/generated/Me';
import { Permission } from '../api/generated/Permission';
import { allows } from '../auth/me';
import type { Messages } from '../i18n/en';

export type NavEntry = {
    to:
        | '/'
        | '/statistics'
        | '/console'
        | '/resources'
        | '/schedule'
        | '/artifacts'
        | '/diagnostics'
        | '/debug/events'
        | '/debug/state-bags'
        | '/debug/http'
        | '/players'
        | '/map'
        | '/bans'
        | '/changes'
        | '/drops'
        | '/settings'
        | '/tools'
        | '/cfg'
        | '/backups'
        | '/audit';
    label: string;
    icon: LucideIcon;
    /**
     * A permission, any one of a list, the owner alone for what no permission grants, or every
     * account.
     */
    access: bigint | bigint[] | 'owner' | 'everyone';
    /** Shows how many alerts are live. */
    alerts?: boolean;
    /**
     * Stays current on the pages under it, as a player's profile is under the players and a resource
     * under the changes, and whatever its address holds, as the drops' filters.
     */
    nested?: boolean;
};

/** Whether the account opens an entry. */
export const opensEntry = (me: Me, { access }: Pick<NavEntry, 'access'>) =>
    access === 'everyone'
        ? true
        : access === 'owner'
          ? me.isOwner
          : (Array.isArray(access) ? access : [access]).some((permission) => allows(me, permission));

/**
 * The panel's pages as the rail lists them, by what one comes for: a person, the server's setup,
 * what the server did, the upkeep. The settings sit at the rail's foot.
 */
export const navSections = (t: Messages) => ({
    main: [
        { to: '/', label: t.nav.overview, icon: LayoutDashboard, access: Permission.DashboardView },
        { to: '/console', label: t.nav.console, icon: SquareTerminal, access: Permission.ConsoleView },
        {
            to: '/resources',
            label: t.nav.resources,
            icon: Blocks,
            access: [Permission.DashboardView, Permission.ResourcesControl],
        },
    ] satisfies NavEntry[],
    people: [
        { to: '/players', label: t.nav.players, icon: Contact, access: Permission.PlayersView, nested: true },
        { to: '/map', label: t.nav.map, icon: MapIcon, access: Permission.PlayersMap },
        { to: '/bans', label: t.nav.bans, icon: Ban, access: Permission.BansView },
    ] satisfies NavEntry[],
    server: [
        {
            to: '/schedule',
            label: t.nav.schedule,
            icon: CalendarClock,
            access: [Permission.DashboardView, Permission.ScheduleManage, Permission.ServerControl],
        },
        { to: '/cfg', label: t.nav.cfg, icon: FileCog, access: 'owner' },
        { to: '/artifacts', label: t.nav.artifacts, icon: Package, access: Permission.ArtifactsView },
        { to: '/backups', label: t.nav.backups, icon: DatabaseBackup, access: Permission.BackupsView },
        {
            to: '/changes',
            label: t.nav.changes,
            icon: GitCompare,
            access: Permission.ChangesView,
            nested: true,
        },
    ] satisfies NavEntry[],
    insights: [
        { to: '/statistics', label: t.nav.statistics, icon: ChartLine, access: Permission.DashboardView },
        { to: '/drops', label: t.nav.drops, icon: Unplug, access: Permission.DropsView, nested: true },
        {
            to: '/diagnostics',
            label: t.nav.diagnostics,
            icon: Stethoscope,
            access: Permission.DiagnosticsView,
            alerts: true,
        },
    ] satisfies NavEntry[],
    // For the day something misbehaves: the rail keeps them folded under one line.
    debug: [
        { to: '/debug/events', label: t.nav.debugEvents, icon: Radar, access: Permission.DiagnosticsView },
        {
            to: '/debug/state-bags',
            label: t.nav.debugBags,
            icon: DatabaseZap,
            access: Permission.DiagnosticsView,
        },
        { to: '/debug/http', label: t.nav.debugHttp, icon: Globe, access: Permission.DiagnosticsView },
    ] satisfies NavEntry[],
    maintenance: [
        { to: '/tools', label: t.nav.tools, icon: Wrench, access: 'owner', nested: true },
        { to: '/audit', label: t.nav.audit, icon: ScrollText, access: Permission.AuditView },
    ] satisfies NavEntry[],
    // Every account opens its own settings, and the categories its permissions open.
    settings: [
        { to: '/settings', label: t.nav.settings, icon: Settings, access: 'everyone', nested: true },
    ] satisfies NavEntry[],
});
