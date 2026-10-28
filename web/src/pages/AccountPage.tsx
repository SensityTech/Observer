import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { KeyRound, LogOut, MonitorSmartphone } from 'lucide-react';
import { useState, type FormEvent } from 'react';

import { ApiError, send } from '../api/client';
import { sessionsQuery } from '../api/users';
import { confirmationProblem, newPasswordProblem, type Problem } from '../auth/validation';
import { Button } from '../components/Button';
import { Chip } from '../components/Chip';
import { PasswordField } from '../components/Field';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { errorMessage, problemMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

export function AccountPage() {
    const { t } = useI18n();

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {t.myAccount.title} <span className="gloss">{t.myAccount.gloss}</span>
                </h1>
            </div>
            <div className="flex flex-col gap-3">
                <PasswordCard />
                <SessionsCard />
            </div>
        </div>
    );
}

type Problems = { current?: Problem | null; next?: Problem | null; confirmation?: Problem | null };

function PasswordCard() {
    const { t } = useI18n();
    const queryClient = useQueryClient();
    const [current, setCurrent] = useState('');
    const [next, setNext] = useState('');
    const [confirmation, setConfirmation] = useState('');
    const [problems, setProblems] = useState<Problems>({});
    const [outcome, setOutcome] = useState<{ tone: 'success' | 'danger'; message: string } | null>(null);

    const change = useMutation({
        mutationFn: () => send('POST', '/account/password', { current, next }),
        onSuccess: () => {
            setCurrent('');
            setNext('');
            setConfirmation('');
            setOutcome({ tone: 'success', message: t.myAccount.password.changed });
            void queryClient.invalidateQueries({ queryKey: sessionsQuery.queryKey });
        },
        onError: (error) => {
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
                setOutcome({ tone: 'danger', message: errorMessage(t, error) });
            }
        },
    });

    const submit = (event: FormEvent) => {
        event.preventDefault();
        const found = {
            current: current === '' ? ({ code: 'required' } as const) : null,
            next: newPasswordProblem(current, next),
            confirmation: confirmationProblem(next, confirmation),
        };
        setProblems(found);
        setOutcome(null);
        if (!found.current && !found.next && !found.confirmation && !change.isPending) {
            change.mutate();
        }
    };

    const message = (problem: Problem | null | undefined) =>
        problem ? problemMessage(t, problem) : undefined;

    return (
        <section className="card" aria-labelledby="account-password">
            <div className="card-head">
                <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                    <KeyRound size={14} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <h2 id="account-password" className="text-card text-ink">
                    {t.myAccount.password.title}
                </h2>
            </div>
            <form className="flex max-w-md flex-col gap-4 p-5 max-md:p-4" onSubmit={submit} noValidate>
                {outcome && <Notice tone={outcome.tone}>{outcome.message}</Notice>}
                <PasswordField
                    label={t.myAccount.password.current}
                    autoComplete="current-password"
                    value={current}
                    error={message(problems.current)}
                    onChange={(event) => {
                        setCurrent(event.target.value);
                        setProblems((found) => ({ ...found, current: null }));
                    }}
                />
                <PasswordField
                    label={t.myAccount.password.next}
                    hint={t.myAccount.password.nextHint}
                    autoComplete="new-password"
                    value={next}
                    error={message(problems.next)}
                    onChange={(event) => {
                        setNext(event.target.value);
                        setProblems((found) => ({ ...found, next: null }));
                    }}
                />
                <PasswordField
                    label={t.myAccount.password.confirm}
                    autoComplete="new-password"
                    value={confirmation}
                    error={message(problems.confirmation)}
                    onChange={(event) => {
                        setConfirmation(event.target.value);
                        setProblems((found) => ({ ...found, confirmation: null }));
                    }}
                />
                <Button
                    type="submit"
                    tone="ink"
                    pending={change.isPending}
                    className="self-start max-md:self-stretch"
                >
                    {t.myAccount.password.submit}
                </Button>
            </form>
        </section>
    );
}

function SessionsCard() {
    const { t, format } = useI18n();
    const queryClient = useQueryClient();
    const sessions = useQuery(sessionsQuery);
    const [outcome, setOutcome] = useState<{ tone: 'success' | 'danger'; message: string } | null>(null);

    const revoke = useMutation({
        mutationFn: (id: number) => send('DELETE', `/account/sessions/${id}`),
        onSuccess: () => setOutcome({ tone: 'success', message: t.myAccount.sessions.revoked }),
        onError: (error) => setOutcome({ tone: 'danger', message: errorMessage(t, error) }),
        onSettled: () => queryClient.invalidateQueries({ queryKey: sessionsQuery.queryKey }),
    });

    return (
        <section className="card flex flex-col" aria-labelledby="account-sessions">
            <div className="card-head wrap-push">
                <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                    <MonitorSmartphone size={14} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <h2 id="account-sessions" className="text-card text-ink">
                    {t.myAccount.sessions.title}
                </h2>
                <span className="text-meta text-ink-3" data-push>
                    {t.myAccount.sessions.meta}
                </span>
            </div>
            {outcome && (
                <div className="px-5 pt-4 max-md:px-4">
                    <Notice tone={outcome.tone}>{outcome.message}</Notice>
                </div>
            )}
            {sessions.isPending && (
                <div className="flex flex-col gap-3 p-5" aria-busy="true">
                    <span className="sr-only">{t.common.loading}</span>
                    <span className="skeleton w-2/3" />
                    <span className="skeleton w-1/2" />
                </div>
            )}
            {sessions.isError && (
                <div className="p-5 max-md:p-4">
                    <Notice tone="danger">
                        {t.myAccount.sessions.loadFailed} {errorMessage(t, sessions.error)}
                    </Notice>
                </div>
            )}
            {sessions.data && (
                <div className={table.scroll}>
                    <table className={table.table}>
                        <caption className="sr-only">{t.myAccount.sessions.caption}</caption>
                        <thead>
                            <tr>
                                <th scope="col" className={table.th}>
                                    {t.myAccount.sessions.started}
                                </th>
                                <th scope="col" className={table.th}>
                                    {t.myAccount.sessions.lastSeen}
                                </th>
                                <th scope="col" className={table.th}>
                                    {t.myAccount.sessions.expires}
                                </th>
                                <th scope="col" className={table.th}>
                                    {t.myAccount.sessions.address}
                                </th>
                                <th scope="col" className={table.th}>
                                    {t.myAccount.sessions.browser}
                                </th>
                                <th scope="col" className={table.th}>
                                    <span className="sr-only">{t.myAccount.sessions.actions}</span>
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            {sessions.data.map((session) => {
                                const started = format.timestamp(session.createdAt, { seconds: false });
                                return (
                                    <tr key={session.id} className={table.row}>
                                        <td className={cn(table.td, table.mono)}>{started}</td>
                                        <td className={cn(table.td, table.mono)}>
                                            {format.timestamp(session.lastSeenAt, { seconds: false })}
                                        </td>
                                        <td className={cn(table.td, table.mono)}>
                                            {format.timestamp(session.expiresAt, { seconds: false })}
                                        </td>
                                        <td className={cn(table.td, table.mono)}>
                                            {session.ip ?? t.myAccount.sessions.unknown}
                                        </td>
                                        <td className={cn(table.td, 'max-w-96 min-w-48')}>
                                            <span
                                                className="block truncate"
                                                title={session.userAgent ?? undefined}
                                            >
                                                {session.userAgent ?? t.myAccount.sessions.unknown}
                                            </span>
                                        </td>
                                        <td className={cn(table.td, 'w-[1%] text-right whitespace-nowrap')}>
                                            {session.current ? (
                                                <Chip>{t.myAccount.sessions.current}</Chip>
                                            ) : (
                                                <Button
                                                    tone="ghost"
                                                    size="sm"
                                                    icon={LogOut}
                                                    pending={
                                                        revoke.isPending && revoke.variables === session.id
                                                    }
                                                    aria-label={t.myAccount.sessions.revokeLabel(started)}
                                                    onClick={() => {
                                                        setOutcome(null);
                                                        revoke.mutate(session.id);
                                                    }}
                                                >
                                                    {t.myAccount.sessions.revoke}
                                                </Button>
                                            )}
                                        </td>
                                    </tr>
                                );
                            })}
                        </tbody>
                    </table>
                </div>
            )}
        </section>
    );
}
