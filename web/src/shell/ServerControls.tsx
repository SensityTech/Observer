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
 * marked disabled; pressed, it says why below, since a touch screen or the keyboard never shows its
 * title. Stop and Restart ask first, since players are disconnected.
 */
export function ServerControls() {
    const { t } = useI18n();
    const me = useMe();
    const { status } = useServerStatus();
    const [confirming, setConfirming] = useState<Action | null>(null);
    const [failure, setFailure] = useState<{ tone: 'warning' | 'danger'; text: string } | null>(null);
    const permitted = allows(me, Permission.ServerControl);

    const label = { start: t.controls.start, stop: t.controls.stop, restart: t.controls.restart };
    const mutation = useMutation({
        mutationFn: (action: Action) => SEND[action](),
        onMutate: () => setFailure(null),
        onError: (error, action) =>
            setFailure({ tone: 'danger', text: t.controls.failed(label[action], errorMessage(t, error)) }),
        onSettled: () => setConfirming(null),
    });

    const running = status !== null && isRunning(status.state);
    const act = (action: Action) => {
        const reason = blocked(t, action, status, permitted);
        if (reason !== null) {
            setFailure({ tone: 'warning', text: t.controls.unavailable(label[action], reason) });
            return;
        }
        setFailure(null);
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
    // No process runs while FXServer waits to restart or sits in a crash loop: a stop only keeps it down.
    const keepsDown = status?.state === 'crashed' || status?.state === 'crashLoop';

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
                <Notice tone={failure.tone} className="controls-failure">
                    <div className="flex items-start gap-3">
                        <p className="flex-1">{failure.text}</p>
                        <button
                            type="button"
                            className="-m-1 rounded-sm p-1"
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
                    title={
                        confirming === 'restart'
                            ? t.controls.restartTitle
                            : keepsDown
                              ? t.controls.keepStoppedTitle
                              : t.controls.stopTitle
                    }
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
                                {confirming === 'restart'
                                    ? t.controls.confirmRestart
                                    : keepsDown
                                      ? t.controls.confirmKeepStopped
                                      : t.controls.confirmStop}
                            </Button>
                        </>
                    }
                >
                    {confirming === 'stop' && keepsDown ? (
                        <p>{t.controls.keepStopped}</p>
                    ) : (
                        <>
                            <p>{t.controls.connected(players)}</p>
                            <p className="text-ink-3">{t.controls.noticeFirst}</p>
                        </>
                    )}
                </Dialog>
            )}
        </>
    );
}
