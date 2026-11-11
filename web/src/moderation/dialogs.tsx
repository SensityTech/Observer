import { useMutation, useQueryClient } from '@tanstack/react-query';
import { LogOut, Megaphone, Send } from 'lucide-react';
import { useEffect, useRef, useState, type ReactNode } from 'react';

import { characters } from '../api/bans';
import {
    announce,
    kickEveryone,
    kickPlayer,
    MAX_KICK_REASON,
    MAX_TEXT,
    messagePlayer,
    type Target,
} from '../api/moderation';
import { onlineQuery } from '../api/players';
import { Button } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { Notice } from '../components/Notice';
import { TextArea } from '../components/TextArea';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { useServerStatus } from '../server/context';

/** How long after its answer the server has acted, and the list of who is connected is read again. */
const ACTED_AFTER_MS = 1_000;

/**
 * A dialog that sends one text about the players connected now, then says what became of it: the
 * server acts a moment after it answers, so the dialog says it rather than leave it to the lists.
 */
function TextDialog<T>({
    title,
    lead,
    label,
    hint,
    missing,
    max,
    submit,
    danger = false,
    icon,
    role,
    send,
    done,
    onClose,
}: {
    title: string;
    lead?: ReactNode;
    label: string;
    hint: string;
    missing: string;
    max: number;
    submit: string;
    danger?: boolean;
    icon: typeof Send;
    role?: 'alertdialog';
    send: (text: string) => Promise<T>;
    done: (answer: T) => string;
    onClose: () => void;
}) {
    const { t } = useI18n();
    const client = useQueryClient();
    const [text, setText] = useState('');
    const [tried, setTried] = useState(false);
    const used = characters(text.trim());
    const wrong = used === 0 || used > max;
    const sending = useMutation({
        mutationFn: () => send(text.trim()),
        onSuccess: () => {
            setTimeout(() => void client.invalidateQueries(onlineQuery), ACTED_AFTER_MS);
        },
    });
    const close = useRef<HTMLButtonElement>(null);
    const sent = sending.isSuccess;
    // The button that sent it is gone: the keyboard goes to the one that is left.
    useEffect(() => {
        if (sent) {
            close.current?.focus();
        }
    }, [sent]);

    if (sending.isSuccess) {
        return (
            <Dialog
                title={title}
                onClose={onClose}
                footer={
                    <Button ref={close} tone="ink" onClick={onClose}>
                        {t.common.close}
                    </Button>
                }
            >
                <Notice tone="success">{done(sending.data)}</Notice>
            </Dialog>
        );
    }
    return (
        <Dialog
            role={role}
            title={title}
            onClose={onClose}
            closeOnBackdrop={text === ''}
            onSubmit={() => {
                setTried(true);
                if (!wrong && !sending.isPending) {
                    sending.mutate();
                }
            }}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button
                        type="submit"
                        tone={danger ? 'danger' : 'ink'}
                        icon={icon}
                        pending={sending.isPending}
                    >
                        {submit}
                    </Button>
                </>
            }
        >
            {lead}
            <TextArea
                label={label}
                rows={3}
                maxLength={max * 2}
                counter={t.moderation.counter(used, max)}
                hint={hint}
                error={tried && wrong ? missing : undefined}
                value={text}
                onChange={(event) => setText(event.target.value)}
            />
            {sending.isError && <Notice tone="danger">{errorMessage(t, sending.error)}</Notice>}
        </Dialog>
    );
}

/** Who a dialog is about: their server ID, and their licence once the panel knows it. */
function Player({ target }: { target: Target }) {
    const { t } = useI18n();
    return (
        <p className="-mt-1.5 font-mono text-small text-ink-3 [overflow-wrap:anywhere]">
            {t.moderation.player(target.netId, target.license)}
        </p>
    );
}

/** Kicks one connected player, for a reason they read in the owner's kick message. */
export function KickDialog({ target, onClose }: { target: Target; onClose: () => void }) {
    const { t } = useI18n();
    const words = t.moderation.kickDialog;
    return (
        <TextDialog
            title={words.title(target.name)}
            lead={<Player target={target} />}
            label={words.reason}
            hint={words.reasonHint}
            missing={words.reasonMissing}
            max={MAX_KICK_REASON}
            submit={words.submit}
            danger
            icon={LogOut}
            send={(reason) => kickPlayer(target, reason)}
            done={() => words.done(target.name)}
            onClose={onClose}
        />
    );
}

/** Kicks everyone connected, for one reason they all read. */
export function KickAllDialog({ onClose }: { onClose: () => void }) {
    const { t } = useI18n();
    const words = t.moderation.kickAll;
    const players = useServerStatus().status?.players?.online ?? 0;
    return (
        <TextDialog
            role="alertdialog"
            title={words.title}
            lead={<p>{words.lead(players)}</p>}
            label={words.reason}
            hint={words.reasonHint}
            missing={words.reasonMissing}
            max={MAX_KICK_REASON}
            submit={words.submit}
            danger
            icon={LogOut}
            send={kickEveryone}
            done={(kicked) => words.done(kicked.players)}
            onClose={onClose}
        />
    );
}

/** Raises an announcement for the server's resources to show to everyone. */
export function AnnounceDialog({ onClose }: { onClose: () => void }) {
    const { t } = useI18n();
    const words = t.moderation.announce;
    return (
        <TextDialog
            title={words.title}
            label={words.text}
            hint={t.moderation.raised('observer:announcement')}
            missing={words.missing}
            max={MAX_TEXT}
            submit={words.submit}
            icon={Megaphone}
            send={announce}
            done={() => words.done}
            onClose={onClose}
        />
    );
}

/** Raises a message for the server's resources to show to one connected player. */
export function MessageDialog({ target, onClose }: { target: Target; onClose: () => void }) {
    const { t } = useI18n();
    const words = t.moderation.messageDialog;
    return (
        <TextDialog
            title={words.title(target.name)}
            lead={<Player target={target} />}
            label={words.text}
            hint={t.moderation.raised('observer:directMessage')}
            missing={words.missing}
            max={MAX_TEXT}
            submit={words.submit}
            icon={Send}
            send={(message) => messagePlayer(target, message)}
            done={() => words.done(target.name)}
            onClose={onClose}
        />
    );
}
