import { useQuery } from '@tanstack/react-query';
import { useState, type ReactNode } from 'react';

import { ApiError, send } from '../api/client';
import type { CreatedUser } from '../api/generated/CreatedUser';
import type { TemporaryPassword } from '../api/generated/TemporaryPassword';
import type { User } from '../api/generated/User';
import { permissionCatalogQuery } from '../api/users';
import { useMe } from '../auth/me';
import { formatPermissions, grantable, parsePermissions } from '../auth/permissions';
import { usernameProblem, type Problem } from '../auth/validation';
import { Button } from '../components/Button';
import { CopyField } from '../components/CopyField';
import { Dialog } from '../components/Dialog';
import { TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage, problemMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { PermissionEditor } from './PermissionEditor';

type Close = { onClose: () => void };

/** The catalog the editor shows, or why it is missing. */
function CatalogEditor({ value, onChange }: { value: bigint; onChange: (value: bigint) => void }) {
    const { t } = useI18n();
    const me = useMe();
    const catalog = useQuery(permissionCatalogQuery);

    if (catalog.isError) {
        return (
            <Notice tone="danger">
                {t.users.editor.loadFailed} {errorMessage(t, catalog.error)}
            </Notice>
        );
    }
    if (catalog.isPending) {
        return (
            <div className="flex flex-col gap-3" aria-busy="true">
                <span className="sr-only">{t.common.loading}</span>
                <span className="skeleton w-2/3" />
                <span className="skeleton w-1/2" />
            </div>
        );
    }
    return (
        <PermissionEditor
            catalog={catalog.data}
            value={value}
            onChange={onChange}
            grantable={grantable(me, catalog.data)}
        />
    );
}

export function CreateUserDialog({
    onClose,
    onCreated,
}: Close & { onCreated: (created: CreatedUser) => void }) {
    const { t } = useI18n();
    const [username, setUsername] = useState('');
    const [permissions, setPermissions] = useState(0n);
    const [problem, setProblem] = useState<Problem | null>(null);
    const [failure, setFailure] = useState<string | null>(null);
    const [pending, setPending] = useState(false);

    const submit = async () => {
        const found = usernameProblem(username);
        setProblem(found);
        if (found || pending) {
            return;
        }
        setPending(true);
        setFailure(null);
        try {
            onCreated(
                await send<CreatedUser>('POST', '/users', {
                    username,
                    permissions: formatPermissions(permissions),
                }),
            );
        } catch (error) {
            setPending(false);
            if (
                error instanceof ApiError &&
                (error.code === 'invalid_username' || error.code === 'name_taken')
            ) {
                setProblem({ code: error.code });
            } else {
                setFailure(errorMessage(t, error));
            }
        }
    };

    return (
        <Dialog
            title={t.users.create.title}
            onClose={onClose}
            onSubmit={submit}
            closeOnBackdrop={username === '' && permissions === 0n}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" pending={pending}>
                        {t.users.create.submit}
                    </Button>
                </>
            }
        >
            {failure && <Notice tone="danger">{failure}</Notice>}
            <TextField
                label={t.users.create.username}
                hint={t.users.create.usernameHint}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                value={username}
                error={problem ? problemMessage(t, problem) : undefined}
                onChange={(event) => {
                    setUsername(event.target.value);
                    setProblem(null);
                }}
            />
            <CatalogEditor value={permissions} onChange={setPermissions} />
        </Dialog>
    );
}

export function EditUserDialog({ user, onClose, onSaved }: Close & { user: User; onSaved: () => void }) {
    const { t } = useI18n();
    const [permissions, setPermissions] = useState(() => parsePermissions(user.permissions));
    const [failure, setFailure] = useState<string | null>(null);
    const [pending, setPending] = useState(false);

    const submit = async () => {
        if (pending) {
            return;
        }
        setPending(true);
        setFailure(null);
        try {
            await send<User>('PATCH', `/users/${user.id}`, {
                permissions: formatPermissions(permissions),
                version: user.version,
            });
            onSaved();
        } catch (error) {
            setPending(false);
            setFailure(errorMessage(t, error));
        }
    };

    return (
        <Dialog
            title={t.users.editDialog.title(user.username)}
            onClose={onClose}
            onSubmit={submit}
            closeOnBackdrop={permissions === parsePermissions(user.permissions)}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" pending={pending}>
                        {t.users.editDialog.submit}
                    </Button>
                </>
            }
        >
            {failure && <Notice tone="danger">{failure}</Notice>}
            <CatalogEditor value={permissions} onChange={setPermissions} />
        </Dialog>
    );
}

/** A confirmation that says what the action does, then runs it. */
function Confirm({
    title,
    body,
    submit,
    tone,
    action,
    onClose,
    children,
    ready = true,
}: Close & {
    title: string;
    body: string;
    submit: string;
    tone: 'ink' | 'danger';
    action: () => Promise<void>;
    children?: ReactNode;
    ready?: boolean;
}) {
    const { t } = useI18n();
    const [failure, setFailure] = useState<string | null>(null);
    const [pending, setPending] = useState(false);

    const run = async () => {
        if (pending || !ready) {
            return;
        }
        setPending(true);
        setFailure(null);
        try {
            await action();
        } catch (error) {
            setPending(false);
            setFailure(errorMessage(t, error));
        }
    };

    return (
        <Dialog
            role="alertdialog"
            title={title}
            onClose={onClose}
            onSubmit={run}
            closeOnBackdrop={children === undefined}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone={tone} pending={pending} disabled={!ready}>
                        {submit}
                    </Button>
                </>
            }
        >
            {failure && <Notice tone="danger">{failure}</Notice>}
            <p>{body}</p>
            {children}
        </Dialog>
    );
}

export function ResetPasswordDialog({
    user,
    onClose,
    onReset,
}: Close & { user: User; onReset: (password: string) => void }) {
    const { t } = useI18n();
    return (
        <Confirm
            title={t.users.resetDialog.title(user.username)}
            body={t.users.resetDialog.body}
            submit={t.users.resetDialog.submit}
            tone="ink"
            onClose={onClose}
            action={async () => {
                const reset = await send<TemporaryPassword>('POST', `/users/${user.id}/reset-password`);
                onReset(reset.temporaryPassword);
            }}
        />
    );
}

export function RevokeSessionsDialog({
    user,
    onClose,
    onRevoked,
}: Close & { user: User; onRevoked: () => void }) {
    const { t } = useI18n();
    return (
        <Confirm
            title={t.users.revokeDialog.title(user.username)}
            body={t.users.revokeDialog.body}
            submit={t.users.revokeDialog.submit}
            tone="ink"
            onClose={onClose}
            action={async () => {
                await send('POST', `/users/${user.id}/revoke-sessions`);
                onRevoked();
            }}
        />
    );
}

/** Irreversible, so the name has to be typed before the button wakes up. */
export function DeleteUserDialog({
    user,
    onClose,
    onDeleted,
}: Close & { user: User; onDeleted: () => void }) {
    const { t } = useI18n();
    const [typed, setTyped] = useState('');
    return (
        <Confirm
            title={t.users.deleteDialog.title(user.username)}
            body={t.users.deleteDialog.body}
            submit={t.users.deleteDialog.submit}
            tone="danger"
            ready={typed === user.username}
            onClose={onClose}
            action={async () => {
                await send('DELETE', `/users/${user.id}`);
                onDeleted();
            }}
        >
            <TextField
                label={t.users.deleteDialog.confirm(user.username)}
                autoComplete="off"
                autoCapitalize="none"
                spellCheck={false}
                inputClassName="font-mono text-ui"
                value={typed}
                onChange={(event) => setTyped(event.target.value)}
            />
        </Confirm>
    );
}

export function TemporaryPasswordDialog({
    title,
    password,
    onClose,
}: Close & {
    title: string;
    password: string;
}) {
    const { t } = useI18n();
    return (
        <Dialog
            title={title}
            onClose={onClose}
            closeOnBackdrop={false}
            footer={
                <Button tone="ink" onClick={onClose}>
                    {t.users.temporaryPassword.done}
                </Button>
            }
        >
            <p>{t.users.temporaryPassword.body}</p>
            <CopyField label={t.users.temporaryPassword.label} value={password} />
        </Dialog>
    );
}
