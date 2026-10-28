import { useEffect, useRef, useState } from 'react';

import { configureSession, send } from '../api/client';
import { Button } from '../components/Button';
import { Dialog } from '../components/Dialog';
import { PasswordField } from '../components/Field';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';

/**
 * Asks for the password when the API wants a recent one, then lets the refused request go again.
 * Requests refused at the same time share one prompt.
 */
export function ReauthDialog() {
    const [open, setOpen] = useState(false);
    const prompt = useRef<{ answer: Promise<boolean>; resolve: (confirmed: boolean) => void } | null>(null);

    useEffect(() => {
        configureSession({
            reauthenticate: () => {
                if (prompt.current === null) {
                    let settle: ((confirmed: boolean) => void) | undefined;
                    const answer = new Promise<boolean>((done) => {
                        settle = done;
                    });
                    prompt.current = { answer, resolve: (confirmed) => settle?.(confirmed) };
                    setOpen(true);
                }
                return prompt.current.answer;
            },
        });
        return () => configureSession({ reauthenticate: async () => false });
    }, []);

    const finish = (confirmed: boolean) => {
        prompt.current?.resolve(confirmed);
        prompt.current = null;
        setOpen(false);
    };

    return open ? <ReauthForm onDone={finish} /> : null;
}

function ReauthForm({ onDone }: { onDone: (confirmed: boolean) => void }) {
    const { t } = useI18n();
    const [password, setPassword] = useState('');
    const [error, setError] = useState<string | null>(null);
    const [pending, setPending] = useState(false);

    const submit = async () => {
        if (password === '') {
            setError(t.validation.required);
            return;
        }
        setPending(true);
        try {
            await send('POST', '/auth/reauth', { password });
            onDone(true);
        } catch (failure) {
            setError(errorMessage(t, failure));
            setPassword('');
            setPending(false);
        }
    };

    return (
        <Dialog
            title={t.reauth.title}
            onClose={() => onDone(false)}
            onSubmit={submit}
            closeOnBackdrop={password === ''}
            footer={
                <>
                    <Button tone="ghost" onClick={() => onDone(false)}>
                        {t.common.cancel}
                    </Button>
                    <Button type="submit" tone="ink" pending={pending}>
                        {t.reauth.submit}
                    </Button>
                </>
            }
        >
            <p>{t.reauth.body}</p>
            <PasswordField
                label={t.reauth.password}
                autoComplete="current-password"
                value={password}
                error={error}
                onChange={(event) => {
                    setPassword(event.target.value);
                    setError(null);
                }}
            />
        </Dialog>
    );
}
