import { Server } from 'lucide-react';
import type { ReactNode } from 'react';

import type { IntegrationServer } from '../api/generated/IntegrationServer';
import { Card } from '../components/Card';
import { useI18n } from '../i18n/I18nProvider';
import { useServerStatus } from '../server/context';
import { ANCHORED } from './Cards';
import { LaunchSettingsLink } from './Notes';

/** Where the card stands on the page, for the contents and for links. */
export const THIS_SERVER = 'this-server';

function Fact({ term, value, children }: { term: string; value: ReactNode; children: ReactNode }) {
    return (
        <div className="flex min-w-0 flex-col gap-0.5">
            <dt className="text-meta text-ink-3">{term}</dt>
            <dd className="flex flex-col gap-0.5">
                <span className="flex items-center gap-2 text-card text-ink">{value}</span>
                <span className="text-small text-pretty text-ink-3">{children}</span>
            </dd>
        </div>
    );
}

/**
 * What the server Observer runs does of the contract, now: the version its bridge speaks, the name
 * the bridge took for this start, and whether txAdmin's events are raised too.
 */
export function ThisServer({
    contractVersion,
    server,
}: {
    contractVersion: number;
    server: IntegrationServer;
}) {
    const { t, format } = useI18n();
    const words = t.docs.server;
    const { status } = useServerStatus();
    const { bridge, starting, txadminEvents, txadminEventsNext: next } = server;
    const startedAt = status?.startedAt ?? null;
    const meta =
        bridge === null
            ? words.stopped
            : startedAt === null
              ? words.running
              : words.startedAt(format.timestamp(startedAt, { seconds: false }));

    let txadmin: { value: string; about: string };
    if (txadminEvents !== null) {
        const about = txadminEvents ? words.txadminOn : words.txadminOff;
        txadmin = {
            value: txadminEvents ? words.on : words.off,
            // The setting was saved since this start: the next one goes by it.
            about: next !== null && next !== txadminEvents ? `${about} ${words.txadminNext(next)}` : about,
        };
    } else if (next !== null) {
        txadmin = { value: next ? words.on : words.off, about: words.txadminFromNext };
    } else {
        txadmin = { value: words.notSet, about: words.notSetAbout };
    }

    return (
        <div id={THIS_SERVER} className={ANCHORED}>
            <Card id="docs-this-server" icon={Server} title={words.title} meta={meta}>
                <dl className="grid grid-cols-3 gap-x-6 gap-y-4 max-md:grid-cols-1">
                    <Fact term={words.contract} value={words.version(contractVersion)}>
                        {words.versionAbout}
                    </Fact>
                    {bridge === null ? (
                        <Fact
                            term={words.bridge}
                            value={
                                <>
                                    <span className="led led-off" aria-hidden="true" />
                                    {words.bridgeStopped}
                                </>
                            }
                        >
                            {words.bridgeStoppedAbout}
                        </Fact>
                    ) : (
                        <Fact
                            term={words.bridge}
                            value={
                                <>
                                    <span
                                        className={starting ? 'led led-pending' : 'led led-ok'}
                                        aria-hidden="true"
                                    />
                                    {starting ? words.bridgeStarting : words.bridgeRunning}
                                </>
                            }
                        >
                            {words.bridgeName(
                                <code
                                    key="name"
                                    className="code-inline whitespace-normal [overflow-wrap:anywhere]"
                                >
                                    {bridge}
                                </code>,
                            )}
                        </Fact>
                    )}
                    <Fact term={words.txadmin} value={txadmin.value}>
                        {txadmin.about} <LaunchSettingsLink />
                    </Fact>
                </dl>
            </Card>
        </div>
    );
}
