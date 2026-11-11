import { keepPreviousData, useInfiniteQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { ArrowRight, Check, ChevronDown, Inbox, Plus, Search, UserCheck } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';

import { useAdmission } from '../admission/context';
import { ApproveLicenceDialog, DenyAllDialog, RemoveApprovalDialog } from '../allowlist/dialogs';
import { allowlistLists, approvedQuery, approveRequest, denyRequest, requestsQuery } from '../api/allowlist';
import { ApiError } from '../api/client';
import type { AllowlistRequest } from '../api/generated/AllowlistRequest';
import type { Approval } from '../api/generated/Approval';
import { Permission } from '../api/generated/Permission';
import { shortLicence } from '../api/players';
import { allows, useMe } from '../auth/me';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { CopyValue } from '../components/CopyValue';
import { TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { errorMessage } from '../i18n/errors';
import { formatClock, zoneDate } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { secondsSince, useNow } from '../lib/useNow';
import { useVisit, ZoneNote } from '../players/words';
import { useSchedule } from '../schedule/context';

/** Refusals of what was typed, said under the search field rather than in place of the lists. */
const SEARCH_REFUSALS = new Set(['address_search', 'invalid_search']);

type Done = { tone: 'success' | 'danger'; message: string };

/** What the lists call an approved player: their name, the staff's note, or their licence cut short. */
const called = (approval: Approval) => approval.name ?? approval.note ?? shortLicence(approval.license);

export function AllowlistPage() {
    const { t } = useI18n();
    const words = t.allowlist;
    const me = useMe();
    const client = useQueryClient();
    const status = useAdmission()?.allowlist ?? null;
    const [draft, setDraft] = useState('');
    const [search, setSearch] = useState('');
    const [adding, setAdding] = useState(false);
    const [denyingAll, setDenyingAll] = useState(false);
    const [removing, setRemoving] = useState<Approval | null>(null);
    const [done, setDone] = useState<Done | null>(null);
    const requests = useInfiniteQuery({ ...requestsQuery(search), placeholderData: keepPreviousData });
    const approved = useInfiniteQuery({ ...approvedQuery(search), placeholderData: keepPreviousData });
    const scheduled = useSchedule().zone;
    const zone = requests.data?.pages[0]?.zone ?? approved.data?.pages[0]?.zone ?? scheduled ?? 'UTC';
    const canManage = allows(me, Permission.AllowlistManage);
    const waiting = requests.data?.pages.flatMap((page) => page.requests) ?? [];
    const approvals = approved.data?.pages.flatMap((page) => page.approved) ?? [];
    const refusal = [requests.error, approved.error].find(
        (error) => error instanceof ApiError && SEARCH_REFUSALS.has(error.code),
    );
    const refused = refusal === undefined || refusal === null ? null : errorMessage(t, refusal);

    // A refused connection or an answer given elsewhere moves the counts: the lists are read again.
    const counted = useRef(status);
    useEffect(() => {
        if (status === null) {
            return;
        }
        const before = counted.current;
        counted.current = status;
        if (before !== null && (before.waiting !== status.waiting || before.approved !== status.approved)) {
            void client.invalidateQueries(allowlistLists);
        }
    }, [client, status]);

    const answer = useMutation({
        mutationFn: async ({ request, approve }: { request: AllowlistRequest; approve: boolean }) => {
            await (approve ? approveRequest(request.code) : denyRequest(request.code));
        },
        onMutate: () => setDone(null),
        onSuccess: (_, { request, approve }) => {
            const who = request.name ?? shortLicence(request.license);
            setDone({
                tone: 'success',
                message: approve ? words.approvedNotice(who) : words.deniedNotice(who),
            });
        },
        onError: (error) => setDone({ tone: 'danger', message: errorMessage(t, error) }),
        onSettled: () => client.invalidateQueries(allowlistLists),
    });
    const answering = (request: AllowlistRequest, approve: boolean) =>
        answer.isPending &&
        answer.variables.request.code === request.code &&
        answer.variables.approve === approve;
    const answers = (request: AllowlistRequest) =>
        canManage
            ? {
                  approve: () => answer.mutate({ request, approve: true }),
                  deny: () => answer.mutate({ request, approve: false }),
                  approving: answering(request, true),
                  denying: answering(request, false),
              }
            : undefined;

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <div className="page-head">
                <h1 className="page-title">
                    {words.title} <span className="gloss">{words.gloss}</span>
                </h1>
                <div className="flex flex-wrap items-center gap-3 max-md:w-full">
                    <ZoneNote zone={zone} />
                    {status !== null &&
                        (status.enabled ? (
                            <Chip tone="success" icon={Check}>
                                {words.on}
                            </Chip>
                        ) : (
                            <Chip>{words.off}</Chip>
                        ))}
                    {canManage && (
                        <Button icon={Plus} className="max-md:w-full" onClick={() => setAdding(true)}>
                            {words.approveLicence}
                        </Button>
                    )}
                </div>
            </div>
            {status?.enabled === false && (
                <Notice live={false}>
                    {words.offNotice}{' '}
                    {allows(me, Permission.SettingsView) && (
                        <Link
                            to="/settings/$category"
                            params={{ category: 'allowlist' }}
                            className="inline-flex items-center gap-1 font-medium text-ink underline underline-offset-2"
                        >
                            {words.openSettings}
                            <ArrowRight size={12} strokeWidth={2} aria-hidden="true" />
                        </Link>
                    )}
                </Notice>
            )}
            {done !== null && <Notice tone={done.tone}>{done.message}</Notice>}
            <form
                className="flex flex-wrap items-start gap-3"
                onSubmit={(event) => {
                    event.preventDefault();
                    setSearch(draft.trim());
                }}
            >
                <TextField
                    label={words.search}
                    hint={words.searchHint}
                    error={refused ?? undefined}
                    type="search"
                    value={draft}
                    spellCheck={false}
                    autoCapitalize="none"
                    autoComplete="off"
                    className="min-w-0 flex-1 basis-72"
                    inputClassName="pl-8.5"
                    after={
                        <Search
                            size={16}
                            strokeWidth={1.5}
                            className="pointer-events-none absolute top-1/2 left-2.75 -translate-y-1/2 text-ink-3"
                            aria-hidden="true"
                        />
                    }
                    onChange={(event) => setDraft(event.target.value)}
                />
                <div className="flex gap-2 pt-6.5 max-md:w-full max-md:pt-0 max-md:[&>button]:flex-1">
                    <Button type="submit" icon={Search}>
                        {words.submit}
                    </Button>
                    {search !== '' && (
                        <Button
                            tone="ghost"
                            onClick={() => {
                                setDraft('');
                                setSearch('');
                            }}
                        >
                            {words.clear}
                        </Button>
                    )}
                </div>
            </form>

            <Card
                id="allowlist-requests"
                icon={Inbox}
                title={words.requests.title}
                meta={status === null ? undefined : words.requests.waiting(status.waiting)}
                aside={
                    canManage && search === '' && waiting.length > 0 ? (
                        <Button tone="ghost" size="sm" onClick={() => setDenyingAll(true)}>
                            {words.requests.denyAll}
                        </Button>
                    ) : undefined
                }
                flush
            >
                <div
                    aria-busy={requests.isFetching}
                    className={cn(requests.isPlaceholderData && 'opacity-50')}
                >
                    {requests.isPending && <Loading label={words.requests.loading} />}
                    {requests.isError && refused === null && (
                        <Failed>
                            {words.requests.failed} {errorMessage(t, requests.error)}
                        </Failed>
                    )}
                    {requests.isSuccess && waiting.length === 0 && (
                        <p className="mx-auto max-w-empty px-6 py-6 text-center text-ui text-ink-3">
                            {search !== ''
                                ? words.requests.noMatch
                                : status?.enabled === true
                                  ? words.requests.emptyOn
                                  : words.requests.emptyOff}
                        </p>
                    )}
                    {waiting.length > 0 && refused === null && (
                        <>
                            <div className={cn(table.scroll, 'rounded-none max-md:hidden')}>
                                <table className={table.table}>
                                    <caption className="sr-only">{words.requests.caption}</caption>
                                    <thead>
                                        <tr>
                                            <th scope="col" className={table.th}>
                                                {words.requests.player}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {words.requests.code}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {words.requests.firstAsked}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {words.requests.lastTry}
                                            </th>
                                            {canManage && (
                                                <th scope="col" className={table.th}>
                                                    <span className="sr-only">{words.requests.answer}</span>
                                                </th>
                                            )}
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {waiting.map((request) => (
                                            <RequestRow
                                                key={request.code}
                                                request={request}
                                                zone={zone}
                                                answers={answers(request)}
                                            />
                                        ))}
                                    </tbody>
                                </table>
                            </div>
                            <ul className="flex flex-col md:hidden">
                                {waiting.map((request) => (
                                    <RequestItem
                                        key={request.code}
                                        request={request}
                                        zone={zone}
                                        answers={answers(request)}
                                    />
                                ))}
                            </ul>
                        </>
                    )}
                </div>
                <Foot
                    kept={words.requests.kept}
                    shown={
                        waiting.length === 0 || refused !== null
                            ? null
                            : requests.hasNextPage
                              ? words.requests.shown(waiting.length)
                              : words.requests.allShown(waiting.length)
                    }
                    more={
                        requests.hasNextPage && refused === null
                            ? {
                                  label: words.requests.loadMore,
                                  pending: requests.isFetchingNextPage,
                                  load: () => void requests.fetchNextPage(),
                              }
                            : null
                    }
                />
            </Card>

            <Card
                id="allowlist-approved"
                icon={UserCheck}
                title={words.approved.title}
                meta={status === null ? undefined : words.approved.players(status.approved)}
                flush
            >
                <div
                    aria-busy={approved.isFetching}
                    className={cn(approved.isPlaceholderData && 'opacity-50')}
                >
                    {approved.isPending && <Loading label={words.approved.loading} />}
                    {approved.isError && refused === null && (
                        <Failed>
                            {words.approved.failed} {errorMessage(t, approved.error)}
                        </Failed>
                    )}
                    {approved.isSuccess && approvals.length === 0 && (
                        <p className="mx-auto max-w-empty px-6 py-6 text-center text-ui text-ink-3">
                            {search === '' ? words.approved.empty : words.approved.noMatch}
                        </p>
                    )}
                    {approvals.length > 0 && refused === null && (
                        <>
                            <div className={cn(table.scroll, 'rounded-none max-md:hidden')}>
                                <table className={table.table}>
                                    <caption className="sr-only">{words.approved.caption}</caption>
                                    <thead>
                                        <tr>
                                            <th scope="col" className={table.th}>
                                                {words.approved.player}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {words.approved.by}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {words.approved.when}
                                            </th>
                                            {canManage && (
                                                <th scope="col" className={table.th}>
                                                    <span className="sr-only">{words.approved.remove}</span>
                                                </th>
                                            )}
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {approvals.map((approval) => (
                                            <ApprovalRow
                                                key={approval.id}
                                                approval={approval}
                                                zone={zone}
                                                onRemove={canManage ? () => setRemoving(approval) : undefined}
                                            />
                                        ))}
                                    </tbody>
                                </table>
                            </div>
                            <ul className="flex flex-col md:hidden">
                                {approvals.map((approval) => (
                                    <ApprovalItem
                                        key={approval.id}
                                        approval={approval}
                                        zone={zone}
                                        onRemove={canManage ? () => setRemoving(approval) : undefined}
                                    />
                                ))}
                            </ul>
                        </>
                    )}
                </div>
                <Foot
                    kept={words.approved.kept}
                    shown={
                        approvals.length === 0 || refused !== null
                            ? null
                            : approved.hasNextPage && status !== null && search === ''
                              ? words.approved.shownOf(approvals.length, status.approved)
                              : approved.hasNextPage
                                ? words.requests.shown(approvals.length)
                                : words.approved.allShown(approvals.length)
                    }
                    more={
                        approved.hasNextPage && refused === null
                            ? {
                                  label: words.approved.loadMore,
                                  pending: approved.isFetchingNextPage,
                                  load: () => void approved.fetchNextPage(),
                              }
                            : null
                    }
                />
            </Card>

            {adding && (
                <ApproveLicenceDialog
                    onClose={() => setAdding(false)}
                    onApproved={() => {
                        setAdding(false);
                        setDone({ tone: 'success', message: words.licenceNotice });
                    }}
                />
            )}
            {denyingAll && waiting[0] !== undefined && (
                <DenyAllDialog
                    waiting={status?.waiting ?? waiting.length}
                    until={waiting[0].lastAt}
                    onClose={() => setDenyingAll(false)}
                    onDenied={(denied) => {
                        setDenyingAll(false);
                        setDone({ tone: 'success', message: words.deniedAllNotice(denied) });
                    }}
                />
            )}
            {removing !== null && (
                <RemoveApprovalDialog
                    approval={removing}
                    who={called(removing)}
                    onClose={() => setRemoving(null)}
                    onRemoved={() => {
                        setDone({ tone: 'success', message: words.removedNotice(called(removing)) });
                        setRemoving(null);
                    }}
                />
            )}
        </div>
    );
}

function Loading({ label }: { label: string }) {
    return (
        <div className="flex flex-col gap-3 p-5">
            <span className="sr-only">{label}</span>
            <span className="skeleton w-3/4" />
            <span className="skeleton w-2/3" />
            <span className="skeleton w-1/2" />
        </div>
    );
}

function Failed({ children }: { children: React.ReactNode }) {
    return (
        <div className="p-5 max-md:p-4">
            <Notice tone="danger">{children}</Notice>
        </div>
    );
}

/** A list's foot: what it keeps, how much of it shows, and the way to more. */
function Foot({
    kept,
    shown,
    more,
}: {
    kept: string;
    shown: string | null;
    more: { label: string; pending: boolean; load: () => void } | null;
}) {
    return (
        <div className="flex flex-wrap items-center gap-2 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3 max-md:pl-4">
            <span className="mr-auto">{kept}</span>
            {shown !== null && <output>{shown}</output>}
            {more !== null && (
                <Button tone="ghost" size="sm" icon={ChevronDown} pending={more.pending} onClick={more.load}>
                    {more.label}
                </Button>
            )}
        </div>
    );
}

/** A try as the lists date it: the time alone with what goes under it today, its date otherwise. */
const useTries = (zone: string) => {
    const { t, locale, format } = useI18n();
    const visit = useVisit(zone);
    const now = useNow(30_000);
    return (instant: string) => {
        const today = zoneDate(instant, zone) === zoneDate(now, zone);
        return {
            today,
            when: today ? formatClock(locale, instant, zone) : visit(instant),
            ago: t.allowlist.ago(format.duration(secondsSince(instant, now))),
        };
    };
};

type Answers = { approve: () => void; deny: () => void; approving: boolean; denying: boolean };

function Asker({ request }: { request: AllowlistRequest }) {
    const { t } = useI18n();
    return (
        <div className="flex min-w-0 flex-col gap-0.5">
            {request.name === null ? (
                <span className="text-ink-2 italic">{t.allowlist.requests.noName}</span>
            ) : (
                <span className="font-medium text-ink [overflow-wrap:anywhere]">{request.name}</span>
            )}
            <CopyValue
                value={request.license}
                shown={shortLicence(request.license)}
                label={t.players.copyLicence(request.license)}
            />
        </div>
    );
}

function AnswerButtons({
    request,
    answers,
    wide,
}: {
    request: AllowlistRequest;
    answers: Answers;
    wide?: boolean;
}) {
    const { t } = useI18n();
    const words = t.allowlist.requests;
    const who = request.name ?? request.code;
    const busy = answers.approving || answers.denying;
    return (
        <>
            <Button
                tone="ink"
                size={wide ? 'md' : 'sm'}
                icon={Check}
                aria-label={words.approveOf(who)}
                pending={answers.approving}
                disabled={busy}
                onClick={answers.approve}
            >
                {words.approve}
            </Button>
            <Button
                tone={wide ? 'secondary' : 'ghost'}
                size={wide ? 'md' : 'sm'}
                aria-label={words.denyOf(who)}
                pending={answers.denying}
                disabled={busy}
                onClick={answers.deny}
            >
                {words.deny}
            </Button>
        </>
    );
}

function RequestRow({
    request,
    zone,
    answers,
}: {
    request: AllowlistRequest;
    zone: string;
    answers?: Answers;
}) {
    const { t } = useI18n();
    const words = t.allowlist;
    const tries = useTries(zone);
    const [first, last] = [tries(request.firstAt), tries(request.lastAt)];
    return (
        <tr className={table.row}>
            <td className={cn(table.td, 'min-w-40 align-top')}>
                <Asker request={request} />
            </td>
            <td className={cn(table.td, 'align-top')}>
                <CopyValue value={request.code} shown={request.code} label={words.requests.copyCode} />
            </td>
            <td className={cn(table.td, 'align-top whitespace-nowrap')}>
                <span className={table.mono}>{first.when}</span>
                {first.today && <div className={table.sub}>{words.today}</div>}
            </td>
            <td className={cn(table.td, 'align-top whitespace-nowrap')}>
                <span className={table.mono}>{last.when}</span>
                <div className={table.sub}>
                    {last.today
                        ? `${last.ago} · ${words.requests.tries(request.tries)}`
                        : words.requests.tries(request.tries)}
                </div>
            </td>
            {answers !== undefined && (
                <td className={cn(table.td, 'align-top')}>
                    <div className="flex justify-end gap-1.5">
                        <AnswerButtons request={request} answers={answers} />
                    </div>
                </td>
            )}
        </tr>
    );
}

function RequestItem({
    request,
    zone,
    answers,
}: {
    request: AllowlistRequest;
    zone: string;
    answers?: Answers;
}) {
    const { t } = useI18n();
    const words = t.allowlist;
    const last = useTries(zone)(request.lastAt);
    return (
        <li className="flex flex-col gap-2 border-b border-line px-4 py-3 last:border-b-0">
            <Asker request={request} />
            <dl className="grid grid-cols-2 gap-x-4 gap-y-2 text-small">
                <div className="flex min-w-0 flex-col gap-0.5">
                    <dt className="text-meta text-ink-3">{words.requests.code}</dt>
                    <dd>
                        <CopyValue
                            value={request.code}
                            shown={request.code}
                            label={words.requests.copyCode}
                        />
                    </dd>
                </div>
                <div className="flex min-w-0 flex-col gap-0.5">
                    <dt className="text-meta text-ink-3">{words.requests.lastTry}</dt>
                    <dd className="text-ink">
                        {last.today ? last.ago : last.when} · {words.requests.tries(request.tries)}
                    </dd>
                </div>
            </dl>
            {answers !== undefined && (
                <div className="flex gap-2 [&>button]:flex-1">
                    <AnswerButtons request={request} answers={answers} wide />
                </div>
            )}
        </li>
    );
}

function Approved({ approval }: { approval: Approval }) {
    const { t } = useI18n();
    return (
        <div className="flex min-w-0 flex-col gap-0.5">
            {approval.name === null ? (
                <span className="text-ink-2 italic [overflow-wrap:anywhere]">
                    {approval.note ?? t.allowlist.approved.noName}
                </span>
            ) : approval.player === null ? (
                <span className="font-medium text-ink [overflow-wrap:anywhere]">{approval.name}</span>
            ) : (
                <Link
                    to="/players/$id"
                    params={{ id: String(approval.player) }}
                    className="font-medium text-ink underline-offset-2 [overflow-wrap:anywhere] hover:underline"
                >
                    {approval.name}
                </Link>
            )}
            {approval.name !== null && approval.note !== null && (
                <span className="text-meta text-ink-3 [overflow-wrap:anywhere]">{approval.note}</span>
            )}
            <CopyValue
                value={approval.license}
                shown={shortLicence(approval.license)}
                label={t.players.copyLicence(approval.license)}
            />
        </div>
    );
}

function ApprovalRow({
    approval,
    zone,
    onRemove,
}: {
    approval: Approval;
    zone: string;
    onRemove?: () => void;
}) {
    const { t } = useI18n();
    const words = t.allowlist.approved;
    const visit = useVisit(zone);
    return (
        <tr className={table.row}>
            <td className={cn(table.td, 'min-w-40 align-top')}>
                <Approved approval={approval} />
            </td>
            <td className={cn(table.td, 'align-top')}>
                <span className="block text-ink">{approval.by}</span>
                <span className="block text-meta text-ink-3">
                    {approval.request === null ? words.ahead : words.fromRequest(approval.request)}
                </span>
            </td>
            <td className={cn(table.td, 'align-top whitespace-nowrap')}>
                <span className={table.mono}>{visit(approval.approvedAt)}</span>
            </td>
            {onRemove !== undefined && (
                <td className={cn(table.td, 'align-top text-right')}>
                    <Button
                        tone="ghost"
                        size="sm"
                        aria-label={words.removeOf(called(approval))}
                        onClick={onRemove}
                    >
                        {words.remove}
                    </Button>
                </td>
            )}
        </tr>
    );
}

function ApprovalItem({
    approval,
    zone,
    onRemove,
}: {
    approval: Approval;
    zone: string;
    onRemove?: () => void;
}) {
    const { t } = useI18n();
    const words = t.allowlist.approved;
    const visit = useVisit(zone);
    return (
        <li className="flex flex-col gap-2 border-b border-line px-4 py-3 last:border-b-0">
            <Approved approval={approval} />
            <dl className="grid grid-cols-2 gap-x-4 gap-y-2 text-small">
                <div className="flex min-w-0 flex-col gap-0.5">
                    <dt className="text-meta text-ink-3">{words.by}</dt>
                    <dd className="text-ink">
                        {approval.by}
                        <span className="block text-ink-3">
                            {approval.request === null ? words.ahead : words.fromRequest(approval.request)}
                        </span>
                    </dd>
                </div>
                <div className="flex min-w-0 flex-col gap-0.5">
                    <dt className="text-meta text-ink-3">{words.when}</dt>
                    <dd className="font-mono text-ink">{visit(approval.approvedAt)}</dd>
                </div>
            </dl>
            {onRemove !== undefined && (
                <Button aria-label={words.removeOf(called(approval))} onClick={onRemove}>
                    {words.remove}
                </Button>
            )}
        </li>
    );
}
