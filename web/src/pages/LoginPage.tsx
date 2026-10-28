import { useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { ArrowRight, KeyRound, Shield, X } from 'lucide-react';
import { useEffect, useRef, useState, type FormEvent } from 'react';

import { signIn } from '../api/auth';
import { ApiError, send } from '../api/client';
import type { LoginResponse } from '../api/generated/LoginResponse';
import { landing } from '../auth/me';
import { Button } from '../components/Button';
import { PasswordField, TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { numberParam } from '../i18n/params';
import { SignInLayout } from '../signin/SignInLayout';
import { useCountdown } from '../signin/useCountdown';

// The refusal shows under the username and describes the password too.
const USERNAME_ID = 'login-username';
const REFUSAL_ID = `${USERNAME_ID}-error`;
const HELP_ID = 'login-forgot';

export function LoginPage() {
    const { t } = useI18n();
    const queryClient = useQueryClient();
    const navigate = useNavigate();
    const usernameInput = useRef<HTMLInputElement>(null);
    const passwordInput = useRef<HTMLInputElement>(null);
    const helpToggle = useRef<HTMLButtonElement>(null);
    const [username, setUsername] = useState('');
    const [password, setPassword] = useState('');
    const [missing, setMissing] = useState({ username: false, password: false });
    // One message for both fields, which never says which one was wrong.
    const [refusal, setRefusal] = useState<string | null>(null);
    const [failure, setFailure] = useState<string | null>(null);
    const [pending, setPending] = useState(false);
    const [helpOpen, setHelpOpen] = useState(false);
    const pause = useCountdown();

    useEffect(() => usernameInput.current?.focus(), []);

    const submit = async (event: FormEvent) => {
        event.preventDefault();
        if (pending || pause.seconds > 0) {
            return;
        }
        const blank = { username: username === '', password: password === '' };
        setMissing(blank);
        if (blank.username || blank.password) {
            return;
        }
        setPending(true);
        setRefusal(null);
        setFailure(null);
        try {
            const response = await send<LoginResponse>('POST', '/auth/login', { username, password });
            signIn(queryClient, response);
            await navigate({ to: response.me.mustChangePassword ? '/password' : landing(response.me) });
        } catch (error) {
            setPending(false);
            setPassword('');
            const code = error instanceof ApiError ? error.code : null;
            if (code === 'too_many_attempts' && error instanceof ApiError) {
                pause.start(numberParam(error.params, 'retryAfter'));
            } else if (code === 'invalid_credentials' || code === 'temporary_password_expired') {
                setRefusal(errorMessage(t, error));
                passwordInput.current?.focus();
            } else {
                setFailure(errorMessage(t, error));
            }
        }
    };

    return (
        <SignInLayout eyebrow={t.login.eyebrow} title={t.login.title} tone={t.login.tone} lead={t.login.lead}>
            <form className="signin-form" aria-label={t.login.form} onSubmit={submit} noValidate>
                {pause.seconds > 0 && <Notice tone="warning">{t.login.throttled(pause.seconds)}</Notice>}
                {failure && <Notice tone="danger">{failure}</Notice>}

                <TextField
                    ref={usernameInput}
                    id={USERNAME_ID}
                    label={t.login.username}
                    name="username"
                    autoComplete="username"
                    autoCapitalize="none"
                    spellCheck={false}
                    inputClassName="h-11"
                    value={username}
                    error={refusal ?? (missing.username ? t.validation.required : undefined)}
                    onChange={(event) => {
                        setUsername(event.target.value);
                        setMissing((current) => ({ ...current, username: false }));
                    }}
                />

                <PasswordField
                    ref={passwordInput}
                    label={t.login.password}
                    labelAside={
                        <button
                            ref={helpToggle}
                            type="button"
                            className="signin-forgot"
                            aria-expanded={helpOpen}
                            aria-controls={HELP_ID}
                            onClick={() => setHelpOpen((open) => !open)}
                        >
                            {t.login.forgot}
                        </button>
                    }
                    name="password"
                    autoComplete="current-password"
                    inputClassName="h-11"
                    value={password}
                    aria-invalid={refusal !== null}
                    describedBy={refusal ? REFUSAL_ID : undefined}
                    error={missing.password ? t.validation.required : undefined}
                    onChange={(event) => {
                        setPassword(event.target.value);
                        setMissing((current) => ({ ...current, password: false }));
                    }}
                    below={
                        <ForgotHelp
                            open={helpOpen}
                            onClose={() => {
                                setHelpOpen(false);
                                helpToggle.current?.focus();
                            }}
                        />
                    }
                />

                <Button
                    type="submit"
                    tone="ink"
                    pending={pending}
                    disabled={pause.seconds > 0}
                    className="h-11 w-full px-4.5 text-[15px]"
                >
                    {pause.seconds > 0 ? t.login.wait(pause.seconds) : t.login.submit}
                    {!pending && pause.seconds === 0 && (
                        <ArrowRight size={16} strokeWidth={1.5} aria-hidden="true" />
                    )}
                </Button>
            </form>

            <div className="flex flex-col gap-2.5 text-ink-3">
                <p className="signin-small">{t.login.noSignUp}</p>
                <p className="signin-xs signin-guard">
                    <Shield size={14} strokeWidth={1.5} aria-hidden="true" />
                    <span>{t.login.guard}</span>
                </p>
            </div>
        </SignInLayout>
    );
}

/** The note "Forgot your password?" folds open, as Aperture's does: 250 ms on height and padding. */
function ForgotHelp({ open, onClose }: { open: boolean; onClose: () => void }) {
    const { t } = useI18n();
    const note = useRef<HTMLDivElement>(null);
    const shown = useRef(open);

    useEffect(() => {
        const element = note.current;
        if (element === null || shown.current === open) {
            return;
        }
        shown.current = open;
        element.getAnimations().forEach((animation) => animation.cancel());
        if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
            element.hidden = !open;
            return;
        }
        element.hidden = false;
        const style = getComputedStyle(element);
        const full = {
            height: `${element.offsetHeight}px`,
            paddingTop: style.paddingTop,
            paddingBottom: style.paddingBottom,
            marginTop: style.marginTop,
        };
        const shut = { height: '0px', paddingTop: '0px', paddingBottom: '0px', marginTop: '0px' };
        element.style.overflow = 'hidden';
        const animation = element.animate(open ? [shut, full] : [full, shut], {
            duration: 250,
            easing: 'cubic-bezier(.4, 0, .2, 1)',
        });
        animation.onfinish = () => {
            element.style.overflow = '';
            element.hidden = !open;
        };
    }, [open]);

    return (
        <div ref={note} id={HELP_ID} className="signin-note" role="note" hidden>
            <span className="signin-note-icon">
                <KeyRound size={16} strokeWidth={1.5} aria-hidden="true" />
            </span>
            <div className="flex min-w-0 flex-1 flex-col gap-1">
                <p className="signin-small font-semibold text-ink">{t.login.forgotTitle}</p>
                <p className="signin-small text-ink-3">
                    {t.login.forgotBody.before}
                    <code className="signin-code">{t.login.forgotBody.code}</code>
                    {t.login.forgotBody.after}
                </p>
                <p className="signin-small text-ink-3">{t.login.forgotAfter}</p>
            </div>
            <Button tone="ghost" size="icon-sm" icon={X} aria-label={t.login.closeHelp} onClick={onClose} />
        </div>
    );
}
