import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link, useNavigate, useParams } from '@tanstack/react-router';
import {
    ArrowLeft,
    ChartColumn,
    Clock,
    Fingerprint,
    History,
    Link2,
    OctagonX,
    Signature,
    UserX,
} from 'lucide-react';
import { useState, type ReactNode } from 'react';

import { ApiError } from '../api/client';
import type { LinkedLicence } from '../api/generated/LinkedLicence';
import type { PlayerDrop } from '../api/generated/PlayerDrop';
import type { PlayerProfile } from '../api/generated/PlayerProfile';
import type { ShownIdentifier } from '../api/generated/ShownIdentifier';
import { fillDays, forgetPlayer, profileQuery, shortLicence, shortToken, totalOf } from '../api/players';
import { useMe } from '../auth/me';
import { ActivityChart } from '../charts/ActivityChart';
import { ATTRIBUTION } from '../charts/PlayersChart';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { CopyValue } from '../components/CopyValue';
import { Dialog } from '../components/Dialog';
import { TextField } from '../components/Field';
import { Notice } from '../components/Notice';
import { table } from '../components/table';
import { errorMessage } from '../i18n/errors';
import { FIGURE, zoneDate } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { Tile, type Fact } from '../overview/Tiles';
import { DROP_ICONS, OnlineState, useVisit, ZoneNote } from '../players/words';

/** The days the activity chart covers, as the API reads them. */
const ACTIVITY_DAYS = 90;

export function PlayerPage() {
    const { t } = useI18n();
    const params = useParams({ strict: false });
    const id = Number(params.id);
    const valid = Number.isSafeInteger(id) && id > 0;
    const profile = useQuery({ ...profileQuery(id), enabled: valid });
    const gone = !valid || (profile.error instanceof ApiError && profile.error.code === 'player_not_found');

    let body: ReactNode;
    if (gone) {
        body = <Notice>{t.errors.player_not_found}</Notice>;
    } else if (profile.isPending) {
        body = (
            <div className="flex flex-col gap-5" aria-busy="true" aria-label={t.players.profile.loading}>
                <div className="skeleton h-7 w-48" />
                <div className="skeleton h-40 w-full rounded-card" />
                <div className="skeleton h-64 w-full rounded-card" />
            </div>
        );
    } else if (profile.isError) {
        body = (
            <Notice tone="danger">
                {t.players.profile.failed} {errorMessage(t, profile.error)}
            </Notice>
        );
    } else {
        body = <Profile profile={profile.data} />;
    }

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <Link
                to="/players"
                className="inline-flex items-center gap-1.5 self-start rounded-xs text-ui text-ink-3 hover:text-ink max-md:min-h-11"
            >
                <ArrowLeft size={14} strokeWidth={1.5} aria-hidden="true" />
                {t.players.profile.back}
            </Link>
            {body}
        </div>
    );
}

function Profile({ profile }: { profile: PlayerProfile }) {
    const { t } = useI18n();
    const words = t.players.profile;
    const me = useMe();
    const visit = useVisit(profile.zone);
    const [forgetting, setForgetting] = useState(false);
    return (
        <>
            <div className="page-head">
                <h1 className="page-title [overflow-wrap:anywhere]">{profile.name}</h1>
                <div className="flex flex-wrap items-center gap-3 max-md:w-full">
                    <ZoneNote zone={profile.zone} />
                    {me.isOwner && (
                        <Button icon={UserX} className="max-md:flex-1" onClick={() => setForgetting(true)}>
                            {words.forget.button}
                        </Button>
                    )}
                </div>
            </div>
            <section className="card p-5 max-md:p-4" aria-label={words.facts}>
                <dl className="grid grid-cols-2 gap-x-6 gap-y-3 md:flex md:flex-wrap md:gap-x-9">
                    <HeaderFact term={t.players.list.state}>
                        <OnlineState online={profile.online} />
                    </HeaderFact>
                    <HeaderFact term={words.licence}>
                        <CopyValue
                            value={profile.license}
                            shown={shortLicence(profile.license)}
                            label={t.players.copyLicence}
                        />
                    </HeaderFact>
                    <HeaderFact term={words.firstSeen}>
                        <span className="font-mono text-small">{visit(profile.firstSeen)}</span>
                    </HeaderFact>
                    <HeaderFact term={words.lastSeen}>
                        <span className="font-mono text-small">{visit(profile.lastSeen)}</span>
                    </HeaderFact>
                    {profile.importedAt !== null && (
                        <div className="col-span-2 self-center">
                            <Chip>{words.importedAt(zoneDate(profile.importedAt, profile.zone))}</Chip>
                        </div>
                    )}
                </dl>
            </section>
            <Tiles profile={profile} />
            <ActivityCard profile={profile} />
            <div className="grid grid-cols-1 gap-5 max-md:gap-4 2xl:grid-cols-2">
                <NamesCard profile={profile} />
                <DropsCard profile={profile} />
            </div>
            {profile.identifiers !== null && (
                <IdentifiersCard profile={profile} identifiers={profile.identifiers} />
            )}
            {profile.linked !== null && <LinkedCard linked={profile.linked} />}
            {forgetting && <ForgetDialog profile={profile} onClose={() => setForgetting(false)} />}
        </>
    );
}

function HeaderFact({ term, children }: { term: string; children: ReactNode }) {
    return (
        <div className="flex min-w-0 flex-col gap-1">
            <dt className="text-meta text-ink-3">{term}</dt>
            <dd className="text-ui text-ink">{children}</dd>
        </div>
    );
}

function Tiles({ profile }: { profile: PlayerProfile }) {
    const { t, format } = useI18n();
    const words = t.players.profile;
    const today = profile.days.find((day) => day.day === profile.today)?.playMs ?? 0;
    const total: Fact[] = [
        { term: words.seenByObserver, detail: format.playTime(profile.playMs + profile.inProgressMs) },
    ];
    if (profile.importedMs > 0) {
        total.push({ term: words.beforeObserver, detail: format.playTime(profile.importedMs) });
    }
    if (profile.inProgressMs > 0) {
        total.push({ term: words.inProgress, detail: format.playTime(profile.inProgressMs) });
    }
    const rate = (value: number | null) => (value === null ? words.unknown : format.number(value, FIGURE));
    return (
        <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
            <Tile
                icon={Clock}
                label={words.total}
                value={format.playTime(totalOf(profile))}
                facts={total}
                trend={null}
            />
            <Tile
                icon={ChartColumn}
                label={words.last7}
                value={format.playTime(profile.last7Ms)}
                facts={[
                    { term: words.last30, detail: format.playTime(profile.last30Ms) },
                    { term: words.todaySoFar, detail: format.playTime(today) },
                ]}
                trend={null}
            />
            <Tile
                icon={History}
                label={words.sessions}
                value={format.number(profile.sessions)}
                facts={
                    profile.sessions > 0
                        ? [
                              {
                                  term: words.average,
                                  detail: format.playTime(profile.playMs / profile.sessions),
                              },
                              { term: words.longest, detail: format.playTime(profile.longestMs) },
                          ]
                        : []
                }
                trend={
                    profile.sessions > 0 ? null : (
                        <p className="self-end text-small text-ink-3">{words.noSession}</p>
                    )
                }
            />
            <Tile
                icon={OctagonX}
                label={words.crashRate}
                value={rate(profile.crashRate)}
                unit={profile.crashRate === null ? undefined : words.perHundredHours}
                facts={[
                    {
                        term: words.serverRate,
                        detail:
                            profile.serverCrashRate === null
                                ? words.unknown
                                : words.rate(rate(profile.serverCrashRate)),
                    },
                    { term: words.crashesSeen, detail: format.number(profile.crashes) },
                ]}
                trend={null}
            />
        </div>
    );
}

function ActivityCard({ profile }: { profile: PlayerProfile }) {
    const { t, format } = useI18n();
    const words = t.players.profile.activity;
    const days = fillDays(profile.days, profile.today, ACTIVITY_DAYS);
    const played = days.filter((day) => day.playMs > 0);
    const today = days.at(-1)?.playMs ?? 0;
    const summary = `${words.summary(
        played.length,
        format.playTime(played.reduce((sum, day) => sum + day.playMs, 0)),
    )} ${words.today(format.playTime(today))}`;
    return (
        <section className="card flex flex-col" aria-labelledby="player-activity">
            <div className="card-head">
                <span className="grid size-6 place-items-center rounded-[6px] border border-line bg-surface-2 text-ink-2">
                    <ChartColumn size={14} strokeWidth={1.5} aria-hidden="true" />
                </span>
                <h2 id="player-activity" className="card-title">
                    {words.title}
                </h2>
                <p className="text-meta text-ink-3">{words.meta}</p>
            </div>
            <figure className="card-body flex flex-col max-md:p-4">
                <ActivityChart days={days} today={profile.today} />
                <figcaption className="mt-4 border-t border-line pt-3 text-ui text-ink-3">
                    {summary}
                </figcaption>
            </figure>
            <footer className="flex flex-wrap items-center gap-x-1 gap-y-1 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-5 text-meta text-ink-3 max-md:pl-4">
                <span className="mr-auto">{t.players.timeConnected}</span>
                <span>
                    {t.charts.attribution}{' '}
                    <a
                        href={ATTRIBUTION}
                        target="_blank"
                        rel="noreferrer"
                        className="underline-offset-2 hover:text-ink-2 hover:underline"
                    >
                        {t.charts.library}
                    </a>
                </span>
            </footer>
        </section>
    );
}

function NamesCard({ profile }: { profile: PlayerProfile }) {
    const { t } = useI18n();
    const words = t.players.profile.names;
    const visit = useVisit(profile.zone);
    return (
        <Card id="player-names" icon={Signature} title={words.title} flush>
            <div className={table.scroll}>
                <table className={table.table}>
                    <caption className="sr-only">{words.caption}</caption>
                    <thead>
                        <tr>
                            <th scope="col" className={table.th}>
                                {words.name}
                            </th>
                            <th scope="col" className={table.th}>
                                {words.firstSeen}
                            </th>
                            <th scope="col" className={table.th}>
                                {words.lastSeen}
                            </th>
                        </tr>
                    </thead>
                    <tbody>
                        {profile.names.map((name) => (
                            <tr key={`${name.name}-${name.firstSeen}`} className={table.row}>
                                <td className={cn(table.td, 'min-w-40 text-ink [overflow-wrap:anywhere]')}>
                                    {name.name}
                                </td>
                                <td className={cn(table.td, table.mono)}>{visit(name.firstSeen)}</td>
                                <td className={cn(table.td, table.mono)}>{visit(name.lastSeen)}</td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            </div>
        </Card>
    );
}

function DropsCard({ profile }: { profile: PlayerProfile }) {
    const { t, format } = useI18n();
    const words = t.players.profile.drops;
    return (
        <Card id="player-drops" icon={OctagonX} title={words.title} flush>
            <div className="flex flex-col gap-1 border-b border-line px-5 py-4 max-md:px-4">
                <dl className="flex flex-wrap gap-x-7 gap-y-2">
                    {(
                        [
                            [words.crashes, profile.crashes],
                            [words.timeouts, profile.timeouts],
                            [words.kicks, profile.kicks],
                        ] as const
                    ).map(([term, value]) => (
                        <div key={term} className="flex flex-col">
                            <dt className="text-meta text-ink-3">{term}</dt>
                            <dd className="text-figure-sm text-ink">{format.number(value)}</dd>
                        </div>
                    ))}
                </dl>
                <p className="text-small text-ink-3">{words.lifetime}</p>
            </div>
            {profile.drops.length === 0 ? (
                <p className="px-6 py-4 text-center text-ui text-ink-3">{words.empty}</p>
            ) : (
                <div className={table.scroll}>
                    <table className={table.table}>
                        <caption className="sr-only">{words.caption}</caption>
                        <thead>
                            <tr>
                                <th scope="col" className={table.th}>
                                    {words.when}
                                </th>
                                <th scope="col" className={table.th}>
                                    {words.what}
                                </th>
                                <th scope="col" className={cn(table.th, 'text-right')}>
                                    {words.length}
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            {profile.drops.map((drop) => (
                                <DropRow key={drop.endedAt} drop={drop} zone={profile.zone} />
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </Card>
    );
}

function DropRow({ drop, zone }: { drop: PlayerDrop; zone: string }) {
    const { t, format } = useI18n();
    const words = t.players.profile.drops;
    const visit = useVisit(zone);
    return (
        <tr className={table.row}>
            <td className={cn(table.td, table.mono, 'align-top')}>{visit(drop.endedAt)}</td>
            <td className={cn(table.td, 'align-top')}>
                <div className="flex flex-wrap items-center gap-x-1.5 gap-y-1">
                    <Chip icon={DROP_ICONS[drop.category]}>{words.categories[drop.category]}</Chip>
                    {drop.resource !== null ? (
                        <span>
                            {words.by} <code className="code-inline">{drop.resource}</code>
                        </span>
                    ) : (
                        drop.kind !== null && <span>{words.kinds[drop.kind]}</span>
                    )}
                </div>
            </td>
            <td className={cn(table.td, 'text-right align-top whitespace-nowrap')}>
                {drop.lengthMs === null ? '' : format.playTime(drop.lengthMs)}
            </td>
        </tr>
    );
}

function IdentifiersCard({
    profile,
    identifiers,
}: {
    profile: PlayerProfile;
    identifiers: ShownIdentifier[];
}) {
    const { t } = useI18n();
    const words = t.players.profile.identifiers;
    const visit = useVisit(profile.zone);
    return (
        <Card id="player-identifiers" icon={Fingerprint} title={words.title} flush>
            {identifiers.length === 0 ? (
                <p className="px-6 py-4 text-center text-ui text-ink-3">{words.empty}</p>
            ) : (
                <div className={table.scroll}>
                    <table className={table.table}>
                        <caption className="sr-only">{words.caption}</caption>
                        <thead>
                            <tr>
                                <th scope="col" className={table.th}>
                                    {words.kind}
                                </th>
                                <th scope="col" className={table.th}>
                                    {words.value}
                                </th>
                                <th scope="col" className={table.th}>
                                    {words.firstSeen}
                                </th>
                                <th scope="col" className={table.th}>
                                    {words.lastSeen}
                                </th>
                                <th scope="col" className={cn(table.th, 'text-right')}>
                                    {words.seen}
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            {identifiers.map((identifier) => (
                                <tr
                                    key={`${identifier.kind}:${identifier.value ?? identifier.firstSeen}`}
                                    className={table.row}
                                >
                                    <td className={cn(table.td, 'align-top whitespace-nowrap text-ink')}>
                                        {words.kinds[identifier.kind]}
                                    </td>
                                    <td className={cn(table.td, 'align-top')}>
                                        <IdentifierValue identifier={identifier} />
                                        {(identifier.imported ||
                                            (identifier.kind !== 'ip' && identifier.otherLicences > 0)) && (
                                            <div className={table.sub}>
                                                {identifier.imported && (
                                                    <span title={words.importedHint}>
                                                        <Chip>{words.imported}</Chip>
                                                        <span className="sr-only">{words.importedHint}</span>
                                                    </span>
                                                )}
                                                {identifier.kind !== 'ip' &&
                                                    identifier.otherLicences > 0 &&
                                                    words.sharedWith(identifier.otherLicences)}
                                            </div>
                                        )}
                                    </td>
                                    <td className={cn(table.td, table.mono, 'align-top')}>
                                        {visit(identifier.firstSeen)}
                                    </td>
                                    <td className={cn(table.td, table.mono, 'align-top')}>
                                        {visit(identifier.lastSeen)}
                                    </td>
                                    <td className={cn(table.td, 'text-right align-top whitespace-nowrap')}>
                                        {words.times(identifier.times)}
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
        </Card>
    );
}

/** An address only as how many licences share it; a token shortened; anything else whole. */
function IdentifierValue({ identifier }: { identifier: ShownIdentifier }) {
    const { t } = useI18n();
    const words = t.players.profile.identifiers;
    if (identifier.kind === 'ip' || identifier.value === null) {
        return <span className="text-ink-2">{words.sameAddress(identifier.otherLicences)}</span>;
    }
    const value = `${identifier.kind}:${identifier.value}`;
    return (
        <CopyValue
            value={value}
            shown={identifier.kind === 'token' ? shortToken(identifier.value) : identifier.value}
            label={words.copy(words.kinds[identifier.kind])}
        />
    );
}

function LinkedCard({ linked }: { linked: LinkedLicence[] }) {
    const { t } = useI18n();
    const words = t.players.profile.linked;
    const kinds = t.players.profile.identifiers.kinds;
    return (
        <Card id="player-linked" icon={Link2} title={words.title} flush>
            {linked.length === 0 ? (
                <p className="px-6 py-4 text-center text-ui text-ink-3">{words.empty}</p>
            ) : (
                <div className={table.scroll}>
                    <table className={table.table}>
                        <caption className="sr-only">{words.caption}</caption>
                        <thead>
                            <tr>
                                <th scope="col" className={table.th}>
                                    {words.player}
                                </th>
                                <th scope="col" className={table.th}>
                                    {words.through}
                                </th>
                                <th scope="col" className={table.th}>
                                    {words.link}
                                </th>
                            </tr>
                        </thead>
                        <tbody>
                            {linked.map((other) => (
                                <tr key={other.player} className={table.row}>
                                    <td className={cn(table.td, 'min-w-40 align-top')}>
                                        <Link
                                            to="/players/$id"
                                            params={{ id: String(other.player) }}
                                            className="font-medium text-ink underline-offset-2 [overflow-wrap:anywhere] hover:underline"
                                        >
                                            {other.name}
                                        </Link>
                                        <div className={cn(table.sub, 'font-mono')}>
                                            {shortLicence(other.license)}
                                        </div>
                                    </td>
                                    <td className={cn(table.td, 'align-top')}>
                                        <ul className="flex flex-col gap-0.5">
                                            {other.through.map((link) => (
                                                <li
                                                    key={`${link.kind}:${link.value ?? ''}`}
                                                    className="flex flex-wrap gap-x-1.5"
                                                >
                                                    <span className="text-ink">{kinds[link.kind]}</span>
                                                    {link.value !== null && (
                                                        <span className="font-mono text-small">
                                                            {link.kind === 'token'
                                                                ? shortToken(link.value)
                                                                : link.value}
                                                        </span>
                                                    )}
                                                    <span className="text-ink-3">
                                                        · {link.strong ? words.strong : words.weak}
                                                    </span>
                                                </li>
                                            ))}
                                        </ul>
                                    </td>
                                    <td className={cn(table.td, 'align-top whitespace-nowrap')}>
                                        <span className="text-ink">
                                            {other.strong ? words.strong : words.weak}
                                        </span>
                                        <div className={table.sub}>
                                            {other.steps > 1 ? words.indirect : words.direct}
                                        </div>
                                    </td>
                                </tr>
                            ))}
                        </tbody>
                    </table>
                </div>
            )}
            <p className="rounded-b-card border-t border-line bg-bg-subtle px-5 py-2.5 text-meta text-ink-3 max-md:px-4">
                {words.note}
            </p>
        </Card>
    );
}

/** The owner forgets a player for an erasure request, typing the licence's first digits. */
function ForgetDialog({ profile, onClose }: { profile: PlayerProfile; onClose: () => void }) {
    const { t } = useI18n();
    const words = t.players.profile.forget;
    const client = useQueryClient();
    const navigate = useNavigate();
    const code = profile.license.replace(/^license:/, '').slice(0, 8);
    const [typed, setTyped] = useState('');
    const forget = useMutation({
        mutationFn: () => forgetPlayer(profile.id),
        onSuccess: async () => {
            await navigate({ to: '/players', state: (previous) => ({ ...previous, forgotten: true }) });
            client.removeQueries({ queryKey: profileQuery(profile.id).queryKey });
            await client.invalidateQueries({ queryKey: ['players'] });
        },
    });
    return (
        <Dialog
            role="alertdialog"
            title={words.title(profile.name)}
            onClose={onClose}
            closeOnBackdrop={typed === ''}
            onSubmit={() => {
                if (typed.trim() === code && !forget.isPending) {
                    forget.mutate();
                }
            }}
            footer={
                <>
                    <Button tone="ghost" onClick={onClose}>
                        {t.common.cancel}
                    </Button>
                    <Button
                        type="submit"
                        tone="danger"
                        icon={UserX}
                        pending={forget.isPending}
                        disabled={typed.trim() !== code}
                    >
                        {words.submit}
                    </Button>
                </>
            }
        >
            <p>{words.lead}</p>
            <ul className="-mt-2 flex list-disc flex-col gap-1 pl-5">
                {words.goes.map((item) => (
                    <li key={item}>{item}</li>
                ))}
            </ul>
            <p>{words.stays}</p>
            <p>{words.backups}</p>
            <TextField
                label={words.confirm(code)}
                value={typed}
                spellCheck={false}
                autoCapitalize="none"
                autoComplete="off"
                inputClassName="font-mono"
                onChange={(event) => setTyped(event.target.value)}
            />
            {forget.isError && <Notice tone="danger">{errorMessage(t, forget.error)}</Notice>}
        </Dialog>
    );
}
