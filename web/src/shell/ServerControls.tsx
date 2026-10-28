import { useMutation } from '@tanstack/react-query';
import { Play, RotateCw, Square, X } from 'lucide-react';
import { useState } from 'react';

import { restartServer, startServer, stopServer } from '../api/server';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useServerStatus } from '../server/context';
import { blocked, isRunning, type Action } from '../server/state';

const SEND = { start: startServer, stop: stopServer, restart: restartServer } satisfies Record<
    Action,
    () => Promise<undefined>
>;

/**
 * Start, Stop and Restart beside the status bar. A button that cannot act now stays in the tab order,
 * marked disabled, with the reason as its description; Stop and Restart ask first, since players are
 * disconnected.
 */
export function ServerControls() {
    const { t } = useI18n();
    const me = useMe();
    const { status } = useServerStatus();
    const [confirming, setConfirming] = useState<Action | null>(null);
    const [failure, setFailure] = useState<{ action: Action; reason: string } | null>(null);
    const permitted = allows(me, Permission.ServerControl);

    const mutation = useMutation({
        mutationFn: (action: Action) => SEND[action](),
        onMutate: () => setFailure(null),
        onError: (error, action) => setFailure({ action, reason: errorMessage(t, error) }),
        onSettled: () => setConfirming(null),
    });

    const running = status !== null && isRunning(status.state);
    const label = { start: t.controls.start, stop: t.controls.stop, restart: t.controls.restart };
    const act = (action: Action) => {
        if (blocked(t, action, status, permitted) !== null) {
            return;
        }
        if (action === 'start') {
            mutation.mutate(action);
        } else {
            setConfirming(action);
        }
    };
    const button = (action: Action, icon: typeof Play, tone: 'ink' | 'secondary') => {
        const reason = blocked(t, action, status, permitted);
        return (
            <Button
                tone={tone}
                icon={icon}
                aria-disabled={reason !== null || undefined}
                title={reason ?? undefined}
                pending={mutation.isPending && mutation.variables === action}
                className={cn(
                    reason !== null && 'cursor-not-allowed opacity-45 shadow-none active:translate-y-0',
                )}
                onClick={() => act(action)}
            >
                {label[action]}
            </Button>
        );
    };
    const players = status?.players?.online ?? 0;

    return (
        <>
            <fieldset className="controls">
                <legend className="sr-only">{t.controls.label}</legend>
                {/* One ink button: the action that fits the state. */}
                {button('start', Play, running ? 'secondary' : 'ink')}
                {button('restart', RotateCw, running ? 'ink' : 'secondary')}
                {button('stop', Square, 'secondary')}
            </fieldset>
            {failure && (
                <Notice tone="danger" className="controls-failure">
                    <div className="flex items-start gap-3">
                        <p className="flex-1">{t.controls.failed(label[failure.action], failure.reason)}</p>
                        <button
                            type="button"
                            className="-m-1 rounded-sm p-1 text-danger-ink"
                            aria-label={t.controls.dismiss}
                            onClick={() => setFailure(null)}
                        >
                            <X size={14} strokeWidth={1.5} aria-hidden="true" />
                        </button>
                    </div>
                </Notice>
            )}
            {confirming !== null && confirming !== 'start' && (
                <Dialog
                    role="alertdialog"
                    title={confirming === 'stop' ? t.controls.stopTitle : t.controls.restartTitle}
                    onClose={() => setConfirming(null)}
                    footer={
                        <>
                            <Button tone="ghost" onClick={() => setConfirming(null)}>
                                {t.common.cancel}
                            </Button>
                            <Button
                                tone={confirming === 'stop' ? 'danger' : 'ink'}
                                icon={confirming === 'stop' ? Square : RotateCw}
                                pending={mutation.isPending}
                                onClick={() => mutation.mutate(confirming)}
                            >
                                {confirming === 'stop' ? t.controls.confirmStop : t.controls.confirmRestart}
                            </Button>
                        </>
                    }
                >
                    <p>{t.controls.connected(players)}</p>
                    <p className="text-ink-3">{t.controls.noticeFirst}</p>
                </Dialog>
            )}
        </>
    );
}
