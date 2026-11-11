import { useMutation } from '@tanstack/react-query';
import { Ellipsis, LogOut, Megaphone, Play, RotateCw, SkipForward, Square, X } from 'lucide-react';
import { useState } from 'react';

import { restartServer, startServer, stopNow, stopServer } from '../api/server';
import { Permission } from '../api/generated/Permission';
import type { StopProgress } from '../api/generated/StopProgress';
import { allows, useMe } from '../auth/me';
import { Button, buttonVariants } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { Menu, type MenuItem } from '../components/Menu';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useNow } from '../lib/useNow';
import { AnnounceDialog, KickAllDialog } from '../moderation/dialogs';
import { useServerStatus } from '../server/context';
import { blocked, isRunning, type Action } from '../server/state';
import { MaintenanceButton } from './Maintenance';

const SEND = { start: startServer, stop: stopServer, restart: restartServer } satisfies Record<
    Action,
    () => Promise<undefined>
>;

/**
 * Restart and Stop at the end of the status card, Start in Restart's place while no process runs,
 * and before them the way into maintenance mode for an account that controls the server.
 * A button that cannot act now stays in the tab order, marked disabled; pressed, it says why below,
 * since a touch screen or the keyboard never shows its title. Stop and Restart ask first, since
 * players are disconnected. While the server runs, a menu holds what reaches every player at once:
 * an announcement, and a kick of everyone, for the accounts that may send them. While resources
 * hold a stop, a line under the card names them.
 */
export function ServerControls() {
    const { t } = useI18n();
    const me = useMe();
    const { status } = useServerStatus();
    const [confirming, setConfirming] = useState<Action | null>(null);
    const [failure, setFailure] = useState<{ tone: 'warning' | 'danger'; text: string } | null>(null);
    const [reaching, setReaching] = useState<'announce' | 'kickAll' | null>(null);
    const permitted = allows(me, Permission.ServerControl);

    // Why a blocked control did nothing holds for the state it was pressed in only: it goes with
    // that state, rather than come back when the server returns to it.
    const state = status?.state ?? null;
    const [shownState, setShownState] = useState(state);
    if (state !== shownState) {
        setShownState(state);
        if (failure?.tone === 'warning') {
            setFailure(null);
        }
    }

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
                size="sm"
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
    const holders = status?.stopHolders ?? [];
    const held = status?.state === 'stopping' && status.stop !== null ? status.stop : null;
    const more: MenuItem[] = running
        ? [
              ...(allows(me, Permission.PlayersAnnounce)
                  ? [
                        {
                            label: t.moderation.announce.menu,
                            icon: Megaphone,
                            onSelect: () => setReaching('announce'),
                        },
                    ]
                  : []),
              ...(permitted
                  ? [
                        {
                            label: t.moderation.kickAll.menu,
                            icon: LogOut,
                            danger: true,
                            onSelect: () => setReaching('kickAll'),
                        },
                    ]
                  : []),
          ]
        : [];
    // No process runs while FXServer waits to restart or sits in a crash loop: a stop only keeps it down.
    const keepsDown = status?.state === 'crashed' || status?.state === 'crashLoop';

    return (
        <>
            <fieldset className="controls">
                <legend className="sr-only">{t.controls.label}</legend>
                {permitted && <MaintenanceButton />}
                {running ? button('restart', RotateCw, 'secondary') : button('start', Play, 'ink')}
                {button('stop', Square, 'secondary')}
                {more.length > 0 && (
                    <Menu
                        label={t.moderation.more}
                        buttonClassName={cn(
                            buttonVariants({ tone: 'secondary', size: 'icon-sm' }),
                            'controls-more',
                        )}
                        items={more}
                        placement="below-end"
                    >
                        <Ellipsis size={16} strokeWidth={1.5} aria-hidden="true" />
                    </Menu>
                )}
            </fieldset>
            {held !== null && held.holding.length > 0 && <StopHold stop={held} permitted={permitted} />}
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
                            {holders.length > 0 && <p className="text-ink-3">{t.controls.heldBy(holders)}</p>}
                        </>
                    )}
                </Dialog>
            )}
            {reaching === 'announce' && <AnnounceDialog onClose={() => setReaching(null)} />}
            {reaching === 'kickAll' && <KickAllDialog onClose={() => setReaching(null)} />}
        </>
    );
}

/**
 * A stop that resources hold through the bridge: who Observer waits for, how long at most, and
 * the way out for whoever may stop the server. It counts down every second, so it is read where
 * it stands and never announced.
 */
function StopHold({ stop, permitted }: { stop: StopProgress; permitted: boolean }) {
    const { t, format } = useI18n();
    const now = useNow();
    const [confirming, setConfirming] = useState(false);
    const quit = useMutation({ mutationFn: stopNow, onSettled: () => setConfirming(false) });
    const left = (Date.parse(stop.limitAt) - now) / 1000;

    return (
        <>
            <Notice live={false} className="controls-hold">
                <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
                    <p className="min-w-0 flex-[1_1_18rem] tabular-nums">
                        {t.controls.waiting(stop.holding)}{' '}
                        {left >= 1 ? t.controls.quitsIn(format.elapsed(left)) : t.controls.quitsNow}
                    </p>
                    {permitted && (
                        <Button
                            size="sm"
                            tone="secondary"
                            icon={SkipForward}
                            pending={quit.isPending}
                            onClick={() => setConfirming(true)}
                        >
                            {t.controls.quitNow}
                        </Button>
                    )}
                </div>
                {quit.isError && (
                    <p className="mt-2 text-danger-ink" role="alert">
                        {t.controls.failed(t.controls.quitNow, errorMessage(t, quit.error))}
                    </p>
                )}
            </Notice>
            {confirming && (
                <Dialog
                    role="alertdialog"
                    title={t.controls.quitNowTitle}
                    onClose={() => setConfirming(false)}
                    footer={
                        <>
                            <Button tone="ghost" onClick={() => setConfirming(false)}>
                                {t.common.cancel}
                            </Button>
                            <Button
                                tone="danger"
                                icon={SkipForward}
                                pending={quit.isPending}
                                onClick={() => quit.mutate()}
                            >
                                {t.controls.quitNow}
                            </Button>
                        </>
                    }
                >
                    <p>{t.controls.quitNowBody(stop.holding)}</p>
                    <p className="text-ink-3">{t.controls.quitNowAudit}</p>
                </Dialog>
            )}
        </>
    );
}
