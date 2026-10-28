import { useQueryClient } from '@tanstack/react-query';
import { Link, useNavigate } from '@tanstack/react-router';
import { ArrowRight, Shield } from 'lucide-react';
import { useEffect, useRef, useState, type FormEvent } from 'react';

import { setupQuery, signIn } from '../api/auth';
import { ApiError, send } from '../api/client';
import type { LoginResponse } from '../api/generated/LoginResponse';
import { confirmationProblem, passwordProblem, usernameProblem, type Problem } from '../auth/validation';
import { Button } from '../components/Button';
import { PasswordField, TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage, problemMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { SignInLayout } from '../signin/SignInLayout';

type Problems = { username?: Problem | null; password?: Problem | null; confirmation?: Problem | null };

/** The first run: the setup token from the printed link's fragment creates the owner account. */
export function SetupPage() {
    const { t } = useI18n();
    const queryClient = useQueryClient();
    const navigate = useNavigate();
    // A fragment never reaches a server or a log; read once, then dropped from the address bar.
    const [token] = useState(() => window.location.hash.slice(1));
    const [username, setUsername] = useState('');
    const [password, setPassword] = useState('');
    const [confirmation, setConfirmation] = useState('');
    const [problems, setProblems] = useState<Problems>({});
    const [failure, setFailure] = useState<{ message: string; signInInstead: boolean } | null>(null);
    const [pending, setPending] = useState(false);
    const firstInput = useRef<HTMLInputElement>(null);

    useEffect(() => firstInput.current?.focus(), []);

    useEffect(() => {
        if (window.location.hash !== '') {
            void navigate({ to: '/setup', hash: undefined, replace: true });
        }
    }, [navigate]);

    const submit = async (event: FormEvent) => {
        event.preventDefault();
        const found = {
            username: usernameProblem(username),
            password: passwordProblem(password),
            confirmation: confirmationProblem(password, confirmation),
        };
        setProblems(found);
        if (found.username || found.password || found.confirmation || pending) {
            return;
        }
        setPending(true);
        setFailure(null);
        try {
            const response = await send<LoginResponse>('POST', '/setup', { token, username, password });
            signIn(queryClient, response);
            queryClient.setQueryData(setupQuery.queryKey, { required: false });
            await navigate({ to: '/' });
        } catch (error) {
            setPending(false);
            const code = error instanceof ApiError ? error.code : null;
            if (code === 'invalid_username' || code === 'name_taken') {
                setProblems({ username: { code } });
            } else if (code === 'password_too_short' || code === 'password_too_long') {
                setProblems({ password: { code, params: error instanceof ApiError ? error.params : {} } });
            } else {
                setFailure({ message: errorMessage(t, error), signInInstead: code === 'already_set_up' });
            }
        }
    };

    const message = (problem: Problem | null | undefined) =>
        problem ? problemMessage(t, problem) : undefined;

    return (
        <SignInLayout eyebrow={t.setup.eyebrow} title={t.setup.title} tone={t.setup.tone} lead={t.setup.lead}>
            <form className="signin-form" aria-label={t.setup.form} onSubmit={submit} noValidate>
                {token === '' && <Notice tone="warning">{t.setup.noToken}</Notice>}
                {failure && (
                    <Notice tone="danger">
                        {failure.message}
                        {failure.signInInstead && (
                            <>
                                {' '}
                                <Link to="/login" className="font-medium underline underline-offset-3">
                                    {t.setup.signInInstead}
                                </Link>
                            </>
                        )}
                    </Notice>
                )}
                <TextField
                    label={t.setup.username}
                    hint={t.setup.usernameHint}
                    autoComplete="username"
                    autoCapitalize="none"
                    spellCheck={false}
                    ref={firstInput}
                    inputClassName="h-11"
                    value={username}
                    error={message(problems.username)}
                    onChange={(event) => {
                        setUsername(event.target.value);
                        setProblems((current) => ({ ...current, username: null }));
                    }}
                />
                <PasswordField
                    label={t.setup.password}
                    hint={t.setup.passwordHint}
                    autoComplete="new-password"
                    inputClassName="h-11"
                    value={password}
                    error={message(problems.password)}
                    onChange={(event) => {
                        setPassword(event.target.value);
                        setProblems((current) => ({ ...current, password: null }));
                    }}
                />
                <PasswordField
                    label={t.setup.confirm}
                    autoComplete="new-password"
                    inputClassName="h-11"
                    value={confirmation}
                    error={message(problems.confirmation)}
                    onChange={(event) => {
                        setConfirmation(event.target.value);
                        setProblems((current) => ({ ...current, confirmation: null }));
                    }}
                />
                <Button
                    type="submit"
                    tone="ink"
                    pending={pending}
                    disabled={token === ''}
                    className="h-11 w-full px-4.5 text-[15px]"
                >
                    {t.setup.submit}
                    {!pending && <ArrowRight size={16} strokeWidth={1.5} aria-hidden="true" />}
                </Button>
            </form>

            <p className="signin-xs signin-guard text-ink-3">
                <Shield size={14} strokeWidth={1.5} aria-hidden="true" />
                <span>
                    {t.setup.linkNote.before}
                    <code className="signin-code">{t.setup.linkNote.code}</code>
                    {t.setup.linkNote.after}
                </span>
            </p>
        </SignInLayout>
    );
}
