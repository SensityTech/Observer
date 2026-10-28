import { useQueryClient } from '@tanstack/react-query';
import { useNavigate } from '@tanstack/react-router';
import { ArrowRight } from 'lucide-react';
import { useEffect, useRef, useState, type FormEvent } from 'react';

import { meQuery, signOut } from '../api/auth';
import { ApiError, send } from '../api/client';
import { confirmationProblem, newPasswordProblem, type Problem } from '../auth/validation';
import { landing } from '../auth/me';
import { Button } from '../components/Button';
import { PasswordField } from '../components/Field';
import { Notice } from '../components/Notice';
import { errorMessage, problemMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { SignInLayout } from '../signin/SignInLayout';

type Problems = { current?: Problem | null; next?: Problem | null; confirmation?: Problem | null };

/** A temporary password must be replaced before anything else: the API refuses the rest until then. */
export function PasswordChangePage() {
    const { t } = useI18n();
    const queryClient = useQueryClient();
    const navigate = useNavigate();
    const [current, setCurrent] = useState('');
    const [next, setNext] = useState('');
    const [confirmation, setConfirmation] = useState('');
    const [problems, setProblems] = useState<Problems>({});
    const [failure, setFailure] = useState<string | null>(null);
    const [pending, setPending] = useState(false);
    const firstInput = useRef<HTMLInputElement>(null);

    useEffect(() => firstInput.current?.focus(), []);

    const submit = async (event: FormEvent) => {
        event.preventDefault();
        const found = {
            current: current === '' ? ({ code: 'required' } as const) : null,
            next: newPasswordProblem(current, next),
            confirmation: confirmationProblem(next, confirmation),
        };
        setProblems(found);
        if (found.current || found.next || found.confirmation || pending) {
            return;
        }
        setPending(true);
        setFailure(null);
        try {
            await send('POST', '/account/password', { current, next });
            const me = await queryClient.fetchQuery(meQuery);
            await navigate({ to: me === null ? '/login' : landing(me) });
        } catch (error) {
            setPending(false);
            const code = error instanceof ApiError ? error.code : null;
            if (code === 'wrong_password') {
                setCurrent('');
                setProblems({ current: { code } });
            } else if (
                code === 'password_too_short' ||
                code === 'password_too_long' ||
                code === 'password_unchanged'
            ) {
                setProblems({ next: { code, params: error instanceof ApiError ? error.params : {} } });
            } else {
                setFailure(errorMessage(t, error));
            }
        }
    };

    const message = (problem: Problem | null | undefined) =>
        problem ? problemMessage(t, problem) : undefined;

    return (
        <SignInLayout
            eyebrow={t.passwordChange.eyebrow}
            title={t.passwordChange.title}
            tone={t.passwordChange.tone}
            lead={t.passwordChange.lead}
        >
            <form className="signin-form" aria-label={t.passwordChange.form} onSubmit={submit} noValidate>
                {failure && <Notice tone="danger">{failure}</Notice>}
                <PasswordField
                    label={t.passwordChange.current}
                    autoComplete="current-password"
                    ref={firstInput}
                    inputClassName="h-11"
                    value={current}
                    error={message(problems.current)}
                    onChange={(event) => {
                        setCurrent(event.target.value);
                        setProblems((found) => ({ ...found, current: null }));
                    }}
                />
                <PasswordField
                    label={t.passwordChange.next}
                    hint={t.passwordChange.nextHint}
                    autoComplete="new-password"
                    inputClassName="h-11"
                    value={next}
                    error={message(problems.next)}
                    onChange={(event) => {
                        setNext(event.target.value);
                        setProblems((found) => ({ ...found, next: null }));
                    }}
                />
                <PasswordField
                    label={t.passwordChange.confirm}
                    autoComplete="new-password"
                    inputClassName="h-11"
                    value={confirmation}
                    error={message(problems.confirmation)}
                    onChange={(event) => {
                        setConfirmation(event.target.value);
                        setProblems((found) => ({ ...found, confirmation: null }));
                    }}
                />
                <Button type="submit" tone="ink" pending={pending} className="h-11 w-full px-4.5 text-[15px]">
                    {t.passwordChange.submit}
                    {!pending && <ArrowRight size={16} strokeWidth={1.5} aria-hidden="true" />}
                </Button>
            </form>

            <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="signin-small text-ink-3">{t.passwordChange.note}</p>
                <Button
                    tone="ghost"
                    size="sm"
                    onClick={() => void signOut(queryClient).finally(() => navigate({ to: '/login' }))}
                >
                    {t.passwordChange.signOut}
                </Button>
            </div>
        </SignInLayout>
    );
}
