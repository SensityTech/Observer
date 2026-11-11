import { useMutation, useQueryClient } from '@tanstack/react-query';
import { Check, Trash2 } from 'lucide-react';
import { useState } from 'react';

import { allowlistLists, approveLicence, denyRequests, MAX_NOTE, removeApproval } from '../api/allowlist';
import { characters } from '../api/bans';
import type { Approval } from '../api/generated/Approval';
import { Button } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';

const LICENCE = /^(license:)?[0-9a-f]{40}$/i;

/** Approves a licence ahead of its player's first visit, with a note for the staff. */
export function ApproveLicenceDialog({
    onClose,
    onApproved,
}: {
    onClose: () => void;
    onApproved: (approval: Approval) => void;
}) {
    const { t } = useI18n();
    const words = t.allowlist.dialog;
    const client = useQueryClient();
    const [licence, setLicence] = useState('');
    const [note, setNote] = useState('');
    const [tried, setTried] = useState(false);
    const licenceWrong = !LICENCE.test(licence.trim());
    const noteWrong = characters(note.trim()) > MAX_NOTE;
    const approve = useMutation({
        mutationFn: () => approveLicence(licence, note),
        onSuccess: async (approval) => {
            await client.invalidateQueries(allowlistLists);
            onApproved(approval);
        },
    });
    const submit = () => {
        setTried(true);
        if (!licenceWrong && !noteWrong && !approve.isPending) {
            approve.mutate();
        }
    };

    return (
        <Dialog
            title={words.title}
            onClose={onClose}
            closeOnBackdrop={licence === '' && note === ''}
            onSubmit={submit}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" icon={Check} pending={approve.isPending}>
                        {words.submit}
                    </Button>
                </>
            }
        >
            <p>{words.lead}</p>
            <TextField
                label={words.licence}
                hint={words.licenceHint}
                error={tried && licenceWrong ? words.licenceWrong : undefined}
                value={licence}
                spellCheck={false}
                autoCapitalize="none"
                autoComplete="off"
                inputClassName="font-mono text-ui"
                onChange={(event) => setLicence(event.target.value)}
            />
            <TextField
                label={
                    <>
                        {words.note} <span className="font-normal text-ink-3">· {t.common.optional}</span>
                    </>
                }
                hint={words.noteHint}
                error={tried && noteWrong ? words.noteTooLong(MAX_NOTE) : undefined}
                value={note}
                autoComplete="off"
                onChange={(event) => setNote(event.target.value)}
            />
            {approve.isError && <Notice tone="danger">{errorMessage(t, approve.error)}</Notice>}
        </Dialog>
    );
}

/** Denies every request the list held when it was read: one made since stays. */
export function DenyAllDialog({
    waiting,
    until,
    onClose,
    onDenied,
}: {
    waiting: number;
    /** The latest try the list showed. */
    until: string;
    onClose: () => void;
    onDenied: (denied: number) => void;
}) {
    const { t } = useI18n();
    const words = t.allowlist.denyAllDialog;
    const client = useQueryClient();
    const deny = useMutation({
        mutationFn: () => denyRequests(until),
        onSuccess: async ({ denied }) => {
            await client.invalidateQueries(allowlistLists);
            onDenied(denied);
        },
    });

    return (
        <Dialog
            role="alertdialog"
            title={words.title(waiting)}
            onClose={onClose}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button tone="danger" pending={deny.isPending} onClick={() => deny.mutate()}>
                        {words.submit}
                    </Button>
                </>
            }
        >
            <p>{words.lead}</p>
            {deny.isError && <Notice tone="danger">{errorMessage(t, deny.error)}</Notice>}
        </Dialog>
    );
}

/** Takes an approval back, after saying what it does to a player who is connected. */
export function RemoveApprovalDialog({
    approval,
    who,
    onClose,
    onRemoved,
}: {
    approval: Approval;
    /** What the list calls them: their name, their note, or their licence cut short. */
    who: string;
    onClose: () => void;
    onRemoved: () => void;
}) {
    const { t } = useI18n();
    const words = t.allowlist.removeDialog;
    const client = useQueryClient();
    const remove = useMutation({
        mutationFn: () => removeApproval(approval.id),
        onSuccess: async () => {
            await client.invalidateQueries(allowlistLists);
            onRemoved();
        },
    });

    return (
        <Dialog
            role="alertdialog"
            title={words.title(who)}
            onClose={onClose}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button
                        tone="danger"
                        icon={Trash2}
                        pending={remove.isPending}
                        onClick={() => remove.mutate()}
                    >
                        {words.submit}
                    </Button>
                </>
            }
        >
            <p>{words.lead}</p>
            {remove.isError && <Notice tone="danger">{errorMessage(t, remove.error)}</Notice>}
        </Dialog>
    );
}
