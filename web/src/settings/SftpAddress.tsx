import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Network } from 'lucide-react';
import { useState } from 'react';

import { ApiError } from '../api/client';
import { featuresQuery, setSftpListen, sftpListenQuery } from '../api/features';
import type { SftpListen } from '../api/generated/SftpListen';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { CopyValue } from '../components/CopyValue';
import { TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { anchorOf } from './catalog';

/** Whether a route of SFTP refused because it has no address, or its port did not open. */
export const sftpNotListening = (error: unknown) =>
    error instanceof ApiError &&
    error.code === 'feature_disabled' &&
    (error.params.held === 'notConfigured' || error.params.held === 'portRefused');

const REACHES = ['every', 'local', 'other'] as const;
type Reach = (typeof REACHES)[number];
const HOSTS = { every: '0.0.0.0', local: '127.0.0.1' } as const;
/** The port Observer suggests, beside the panel's. */
const SUGGESTED_PORT = '40126';

type Fields = { reach: Reach; host: string; port: string };

/** `0.0.0.0:40126` in its two parts; an IPv6 host loses its brackets. */
export const splitListen = (listen: string) => {
    const at = listen.lastIndexOf(':');
    return { host: listen.slice(0, at).replace(/^\[|\]$/g, ''), port: listen.slice(at + 1) };
};

/** The two parts as the address Observer takes, or `null` while they cannot make one. */
export const joinListen = (host: string, port: string) => {
    const address = host.trim();
    const digits = port.trim();
    const number = Number(digits);
    if (address === '' || !/^\d{1,5}$/.test(digits) || number < 1 || number > 65_535) {
        return null;
    }
    return address.includes(':') ? `[${address}]:${number}` : `${address}:${number}`;
};

const fieldsOf = (listen: string | null): Fields => {
    if (listen === null) {
        return { reach: 'every', host: '', port: SUGGESTED_PORT };
    }
    const { host, port } = splitListen(listen);
    if (host === HOSTS.every || host === HOSTS.local) {
        return { reach: host === HOSTS.every ? 'every' : 'local', host: '', port };
    }
    return { reach: 'other', host, port };
};

/**
 * Where SFTP listens, for the owner: the address in force and what became of its port, then the
 * form that opens, moves or closes it at once. `observer.toml` shows through when it names the
 * address, and then nothing is changed from here.
 */
export function SftpAddress() {
    const { t } = useI18n();
    const words = t.settings.sftpAddress;
    const me = useMe();
    const client = useQueryClient();
    const reads = me.isOwner || allows(me, Permission.SettingsView);
    const sftp = useQuery({ ...sftpListenQuery, enabled: reads });
    const features = useQuery({ ...featuresQuery, enabled: reads });
    const [draft, setDraft] = useState<Fields | null>(null);
    const save = useMutation({
        mutationFn: setSftpListen,
        onSuccess: (now) => {
            client.setQueryData(sftpListenQuery.queryKey, now);
            setDraft(null);
            void client.invalidateQueries({ queryKey: featuresQuery.queryKey, exact: true });
            void client.invalidateQueries({ queryKey: ['files'] });
        },
    });
    if (!reads) {
        return null;
    }

    const current = sftp.data;
    const fields = draft ?? fieldsOf(current?.listen ?? null);
    const listen = joinListen(fields.reach === 'other' ? fields.host : HOSTS[fields.reach], fields.port);
    const same = current !== undefined && listen === current.listen;
    const feature = features.data?.features.find((view) => view.feature === 'sftp');
    const kept = feature?.held === 'host' ? 'locked' : feature?.enabled === false ? 'off' : 'closed';

    return (
        <Card id={anchorOf('sftpAddress')} icon={Network} title={t.settings.items.sftpAddress.label}>
            <div className="flex flex-col gap-4">
                {sftp.isError && <Notice tone="danger">{errorMessage(t, sftp.error)}</Notice>}
                {sftp.isPending && <div className="skeleton h-9.5 rounded-control" aria-busy="true" />}
                {current !== undefined && <Port sftp={current} kept={kept} />}
                {current?.pinned === false && me.isOwner && (
                    <form
                        className="flex flex-col gap-4"
                        onSubmit={(event) => {
                            event.preventDefault();
                            if (listen !== null) {
                                save.mutate(listen);
                            }
                        }}
                    >
                        <div className="flex flex-col gap-1.5">
                            <Segmented
                                label={words.reach}
                                choices={REACHES}
                                value={fields.reach}
                                words={words.reaches}
                                onChange={(reach) => setDraft({ ...fields, reach })}
                                className="self-start"
                            />
                            <p className="text-small text-pretty text-ink-2">
                                {words.reachAbout[fields.reach]}
                            </p>
                        </div>
                        <div className="flex flex-wrap items-start gap-3">
                            {fields.reach === 'other' && (
                                <TextField
                                    className="flex-[2_1_14rem]"
                                    label={words.host}
                                    hint={words.hostHint}
                                    value={fields.host}
                                    spellCheck={false}
                                    autoComplete="off"
                                    inputClassName="font-mono"
                                    onChange={(event) => setDraft({ ...fields, host: event.target.value })}
                                />
                            )}
                            <TextField
                                className="flex-[0_1_17rem]"
                                label={words.port}
                                hint={words.portHint}
                                value={fields.port}
                                inputMode="numeric"
                                autoComplete="off"
                                inputClassName="font-mono"
                                onChange={(event) => setDraft({ ...fields, port: event.target.value })}
                            />
                        </div>
                        {save.isError && <Notice tone="danger">{errorMessage(t, save.error)}</Notice>}
                        <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
                            <Button
                                type="submit"
                                tone="ink"
                                pending={save.isPending && save.variables !== null}
                                disabled={
                                    listen === null || save.isPending || (same && current.refused === null)
                                }
                            >
                                {current.listen === null
                                    ? words.open
                                    : same && current.refused !== null
                                      ? words.again
                                      : words.move}
                            </Button>
                            {current.listen !== null && (
                                <Button
                                    pending={save.isPending && save.variables === null}
                                    disabled={save.isPending}
                                    onClick={() => save.mutate(null)}
                                >
                                    {words.close}
                                </Button>
                            )}
                            <span className="text-small text-ink-3">{words.applies}</span>
                        </div>
                    </form>
                )}
            </div>
        </Card>
    );
}

/** The address in force and what became of its port, in a line with its LED. */
function Port({ sftp, kept }: { sftp: SftpListen; kept: 'locked' | 'off' | 'closed' }) {
    const { t } = useI18n();
    const words = t.settings.sftpAddress;
    const { listen } = sftp;
    if (listen === null) {
        return <State led="led-warning">{words.none}</State>;
    }
    if (sftp.refused !== null) {
        return (
            <Notice tone="danger">
                {words.refused(listen)}{' '}
                <span className="[overflow-wrap:anywhere]">{words.said(sftp.refused)}</span>
            </Notice>
        );
    }
    return (
        <div className="flex flex-col gap-2">
            {sftp.open ? (
                <State led="led-ok">{words.listening(listen)}</State>
            ) : (
                <State led="led-off">{words[kept](listen)}</State>
            )}
            {sftp.fingerprint !== null && (
                // Whole, to compare with what a client shows at its first connection.
                <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1 text-small text-ink-2">
                    {words.key}
                    <span className="min-w-0 font-mono text-ink [font-variant-ligatures:none] [overflow-wrap:anywhere]">
                        {sftp.fingerprint}
                    </span>
                    <CopyValue value={sftp.fingerprint} shown={t.common.copy} label={words.copyKey} />
                </div>
            )}
            {sftp.pinned && <p className="text-small text-pretty text-ink-2">{words.pinned}</p>}
        </div>
    );
}

function State({ led, children }: { led: string; children: string }) {
    return (
        <p className="flex items-start gap-2 text-body text-ink">
            <span className={cn('led mt-1.75', led)} aria-hidden="true" />
            <span className="min-w-0 [overflow-wrap:anywhere]">{children}</span>
        </p>
    );
}
