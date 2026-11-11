import { Link } from '@tanstack/react-router';
import { AppWindow, Server, type LucideIcon } from 'lucide-react';
import type { ReactNode } from 'react';

import type { IntegrationServer } from '../api/generated/IntegrationServer';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { useI18n } from '../i18n/I18nProvider';
import { anchorOf as settingAnchor } from '../settings/catalog';

/**
 * The places of the contract the panel adds a word to, by their address on the page: where the
 * effect of a resource's code shows in the panel, and what this server does. A test holds each
 * address to the contract, so that a heading renamed there does not lose its note in silence.
 */
export const NOTED = ['player-labels', 'observerstartchanges', 'bans', 'txadmins-events'] as const;

export const link =
    'font-medium text-ink-2 underline decoration-line-strong underline-offset-3 hover:text-ink';

/** The link to the launch setting that sends txAdmin's events, for the owner, who alone opens it. */
export function LaunchSettingsLink() {
    const { t } = useI18n();
    return useMe().isOwner ? (
        <Link
            to="/settings/$category"
            params={{ category: 'launch' }}
            hash={settingAnchor('txadminEvents')}
            className={link}
        >
            {t.docs.server.launchSettings}
        </Link>
    ) : null;
}

/** What the panel says under a section of the contract, if it has a word for it. */
export function SectionNote({ id, server }: { id: string; server: IntegrationServer | null }) {
    const { t } = useI18n();
    const me = useMe();
    const words = t.docs.notes;
    // A page the account does not open is named, not linked.
    const page = (to: '/players' | '/changes' | '/bans', name: string, permission: bigint) =>
        allows(me, permission) ? (
            <Link key={to} to={to} className={link}>
                {name}
            </Link>
        ) : (
            name
        );

    let icon: LucideIcon = AppWindow;
    let lead = words.panel;
    let text: ReactNode;
    switch (id as (typeof NOTED)[number]) {
        case 'player-labels':
            text = words.labels(page('/players', t.nav.players, Permission.PlayersView));
            break;
        case 'observerstartchanges':
            text = words.startChanges(page('/changes', t.nav.changes, Permission.ChangesView));
            break;
        case 'bans':
            text = words.bans(
                page('/bans', t.nav.bans, Permission.BansView),
                allows(me, Permission.SettingsView) ? (
                    <Link
                        key="message"
                        to="/settings/$category"
                        params={{ category: 'bans' }}
                        hash={settingAnchor('refusalMessage')}
                        className={link}
                    >
                        {words.refusalMessage}
                    </Link>
                ) : (
                    words.refusalMessage
                ),
            );
            break;
        case 'txadmins-events': {
            // An account that is not told what the server does reads the contract alone.
            if (server === null) {
                return null;
            }
            const said =
                server.txadminEvents !== null
                    ? server.txadminEvents
                        ? words.txadminOn
                        : words.txadminOff
                    : server.txadminEventsNext !== null
                      ? words.txadminStopped(server.txadminEventsNext)
                      : null;
            if (said === null) {
                return null;
            }
            icon = Server;
            lead = words.server;
            text = (
                <>
                    {said} <LaunchSettingsLink />
                </>
            );
            break;
        }
        default:
            return null;
    }
    const Icon = icon;
    return (
        <footer className="flex items-start gap-2.5 rounded-b-card border-t border-line bg-bg-subtle px-5 py-2.5 text-small text-pretty text-ink-3 max-md:px-4">
            <Icon size={14} strokeWidth={1.5} className="mt-0.75 flex-none" aria-hidden="true" />
            <p className="min-w-0">
                <span className="font-medium text-ink-2">{lead}</span> {text}
            </p>
        </footer>
    );
}
