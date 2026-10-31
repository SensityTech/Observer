import { Link } from '@tanstack/react-router';
import { ListFilter, OctagonX } from 'lucide-react';
import { useState, type ReactNode } from 'react';

import type { CrashRollup } from '../api/generated/CrashRollup';
import type { RepeatCrasher } from '../api/generated/RepeatCrasher';
import type { SignatureEntry } from '../api/generated/SignatureEntry';
import type { SignatureTable } from '../api/generated/SignatureTable';
import { Button } from '../components/Button';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { table } from '../components/table';
import { zoneDate } from '../i18n/format';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { PlayerName } from '../players/words';
import { formatShare, HintChip, keptSince, ShareBar, VerdictChip } from './words';

type Tab = 'signatures' | 'modules' | 'builds';

const TABS: Tab[] = ['signatures', 'modules', 'builds'];

const number = cn(table.td, 'w-[1%] text-right font-semibold whitespace-nowrap text-ink');

/**
 * The crashes grouped by signature, by module or by game build. A signature opens its redacted
 * example texts, written as plain text: a client wrote them. A row's actions sit under its
 * signature, so the table fits its card.
 */
export function SignaturesCard({
    signatures,
    failed,
    stale,
    zone,
    windows,
    rangeFrom,
    repeat,
    repeatSince,
    seesPlayers,
    onShowInLog,
}: {
    signatures: SignatureTable | undefined;
    failed: boolean;
    /** Still the previous range's, while the new one is read. */
    stale: boolean;
    zone: string;
    /** Which windows the verdicts compared. */
    windows: string;
    /** The range's start, to tell when players are counted over a shorter stretch. */
    rangeFrom: string | undefined;
    repeat: RepeatCrasher[];
    /** The day repeat crashers are counted from, when sessions kept start within their 7 days. */
    repeatSince: string | null;
    /** Whether the viewer sees players: a statistics viewer only gets examples two players shared. */
    seesPlayers: boolean;
    /** Narrows the log to a signature; only for a viewer who sees the log. */
    onShowInLog?: (signature: number) => void;
}) {
    const { t, locale } = useI18n();
    const words = t.drops.signatures;
    const [tab, setTab] = useState<Tab>('signatures');
    const since = signatures === undefined ? null : keptSince(locale, signatures.keptFrom, rangeFrom, zone);
    let body: ReactNode;
    if (signatures === undefined) {
        body = failed ? (
            <div className="p-5 max-md:p-4">
                <Notice tone="danger" live={false}>
                    {words.failed}
                </Notice>
            </div>
        ) : (
            <div className="flex flex-col gap-3 p-5" aria-busy="true">
                <span className="sr-only">{words.loading}</span>
                <span className="skeleton w-3/4" />
                <span className="skeleton w-1/2" />
            </div>
        );
    } else if (signatures.crashes === 0) {
        body = <p className="px-6 py-4 text-center text-ui text-ink-3">{words.empty}</p>;
    } else if (tab === 'signatures') {
        body = (
            <SignatureList
                entries={signatures.signatures}
                zone={zone}
                windows={windows}
                seesPlayers={seesPlayers}
                onShowInLog={onShowInLog}
            />
        );
    } else {
        body = <RollupTable rows={tab === 'modules' ? signatures.modules : signatures.builds} tab={tab} />;
    }
    return (
        <Card
            id="drops-signatures"
            icon={OctagonX}
            title={words.title}
            meta={
                signatures === undefined
                    ? undefined
                    : [words.meta(signatures.crashes), since === null ? null : t.drops.playersSince(since)]
                          .filter((part) => part !== null)
                          .join(' · ')
            }
            aside={
                <Segmented
                    label={words.groupBy}
                    choices={TABS}
                    value={tab}
                    words={words.tabs}
                    onChange={setTab}
                    className="md:-my-1.5"
                />
            }
            flush
        >
            <div aria-busy={stale} className={cn('transition-opacity', stale && 'opacity-50')}>
                {body}
            </div>
            <RepeatCrashers repeat={repeat} since={repeatSince} />
        </Card>
    );
}

/**
 * A signature in mono; how it was read, its code name, module and game build; why it has no
 * example; then its actions. Or why it stays hidden.
 */
function SignatureCell({
    entry,
    actions,
    seesPlayers,
}: {
    entry: SignatureEntry;
    actions: ReactNode;
    seesPlayers: boolean;
}) {
    const { t } = useI18n();
    const words = t.drops.signatures;
    if (entry.signature === null) {
        return (
            <div className="flex min-w-0 flex-col gap-0.75">
                <span className="text-ink-2">{words.hidden}</span>
                <div className={table.sub}>
                    <Chip>{words.forms[entry.form]}</Chip>
                    <span>{words.hiddenHint}</span>
                </div>
                {actions}
            </div>
        );
    }
    const facts = [
        words.forms[entry.form],
        entry.module,
        entry.gameBuild === null ? null : words.gameBuild(entry.gameBuild),
        entry.resource === null ? null : words.asset(entry.resource),
    ].filter((fact) => fact !== null);
    return (
        <div className="flex min-w-0 flex-col gap-0.75">
            <span className={cn(table.mono, 'whitespace-normal [overflow-wrap:anywhere]')}>
                {entry.signature}
            </span>
            {entry.label !== null && <span className="font-mono text-meta text-ink-2">{entry.label}</span>}
            <div className={table.sub}>{facts.join(' · ')}</div>
            {entry.examples.length === 0 && (
                <div className={table.sub}>{seesPlayers ? words.noExamples : words.noSharedExample}</div>
            )}
            {actions}
        </div>
    );
}

function FirstSeen({ entry, zone }: { entry: SignatureEntry; zone: string }) {
    const { t } = useI18n();
    return (
        <span className="inline-flex flex-wrap items-center gap-1.5 whitespace-nowrap">
            <span className="font-mono text-small text-ink">{zoneDate(entry.firstSeen, zone)}</span>
            {entry.new && entry.signature !== null && (
                <HintChip hint={t.drops.signatures.newHint}>{t.drops.signatures.new}</HintChip>
            )}
        </span>
    );
}

function SignatureList({
    entries,
    zone,
    windows,
    seesPlayers,
    onShowInLog,
}: {
    entries: SignatureEntry[];
    zone: string;
    windows: string;
    seesPlayers: boolean;
    onShowInLog?: (signature: number) => void;
}) {
    const { t, locale, format } = useI18n();
    const words = t.drops.signatures;
    const [open, setOpen] = useState<number | null>(null);
    const ranked = entries.toSorted((a, b) => b.drops - a.drops);
    const actions = (entry: SignatureEntry) => {
        const examples = entry.signature !== null && entry.examples.length > 0;
        if (!examples && onShowInLog === undefined) {
            return null;
        }
        return (
            <div className="-ml-2.5 flex flex-wrap gap-1 max-md:-ml-0 max-md:gap-2">
                {examples && (
                    <Button
                        size="sm"
                        tone="ghost"
                        className="h-7"
                        aria-expanded={open === entry.id}
                        onClick={() => setOpen(open === entry.id ? null : entry.id)}
                    >
                        {open === entry.id ? words.hideExamples : words.examples}
                    </Button>
                )}
                {onShowInLog !== undefined && (
                    <Button
                        size="sm"
                        tone="ghost"
                        icon={ListFilter}
                        className="h-7"
                        onClick={() => onShowInLog(entry.id)}
                    >
                        {words.showInLog}
                    </Button>
                )}
            </div>
        );
    };
    return (
        <>
            <div className={cn(table.scroll, 'rounded-none max-md:hidden')}>
                <table className={table.table}>
                    <caption className="sr-only">{words.caption}</caption>
                    <thead>
                        <tr>
                            <th scope="col" className={table.th}>
                                {words.signature}
                            </th>
                            <th scope="col" className={cn(table.th, 'text-right')}>
                                {words.crashes}
                            </th>
                            <th scope="col" className={cn(table.th, 'w-[22%]')}>
                                <span className="sr-only">{words.share}</span>
                            </th>
                            <th scope="col" className={cn(table.th, 'text-right')}>
                                {words.players}
                            </th>
                            <th scope="col" className={table.th}>
                                {words.firstSeen}
                            </th>
                            <th scope="col" className={table.th}>
                                {words.verdict}
                            </th>
                        </tr>
                    </thead>
                    <tbody>
                        {ranked.map((entry, index) => (
                            <SignatureRows key={entry.id} entry={entry} open={open === entry.id}>
                                <td className={cn(table.td, 'min-w-56 align-top')}>
                                    <SignatureCell
                                        entry={entry}
                                        actions={actions(entry)}
                                        seesPlayers={seesPlayers}
                                    />
                                </td>
                                <td className={cn(number, 'align-top')}>{format.number(entry.drops)}</td>
                                <td className={cn(table.td, 'align-top')}>
                                    <div className="flex items-center gap-2.5 pt-1.5">
                                        <ShareBar share={entry.share} first={index === 0} />
                                        <span className="w-10 text-right text-small">
                                            {formatShare(locale, entry.share)}
                                        </span>
                                    </div>
                                </td>
                                <td className={cn(table.td, 'text-right align-top')}>
                                    {format.number(entry.players)}
                                </td>
                                <td className={cn(table.td, 'align-top')}>
                                    <FirstSeen entry={entry} zone={zone} />
                                </td>
                                <td className={cn(table.td, 'align-top')}>
                                    <VerdictChip trend={entry.trend} windows={windows} />
                                </td>
                            </SignatureRows>
                        ))}
                    </tbody>
                </table>
            </div>
            <ul className="flex flex-col md:hidden">
                {ranked.map((entry, index) => (
                    <li key={entry.id} className="flex flex-col gap-1.5 border-b border-line px-4 py-3">
                        <SignatureCell entry={entry} actions={null} seesPlayers={seesPlayers} />
                        <div className="flex items-center gap-2.5">
                            <ShareBar share={entry.share} first={index === 0} />
                            <span className="text-small whitespace-nowrap text-ink">
                                {words.meta(entry.drops)} · {formatShare(locale, entry.share)}
                            </span>
                        </div>
                        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-small text-ink-2">
                            <span>{words.playersCount(entry.players)}</span>
                            <FirstSeen entry={entry} zone={zone} />
                            <VerdictChip trend={entry.trend} windows={windows} />
                        </div>
                        {actions(entry)}
                        {open === entry.id && <Examples entry={entry} />}
                    </li>
                ))}
            </ul>
        </>
    );
}

/** A signature's row, and its examples under it once opened. */
function SignatureRows({
    entry,
    open,
    children,
}: {
    entry: SignatureEntry;
    open: boolean;
    children: ReactNode;
}) {
    return (
        <>
            <tr className={cn(table.row, open && '[&>td]:border-b-0')}>{children}</tr>
            {open && (
                <tr>
                    <td colSpan={6} className={cn(table.td, 'bg-bg-subtle')}>
                        <Examples entry={entry} />
                    </td>
                </tr>
            )}
        </>
    );
}

/** The latest redacted crash texts, as plain text, and the share that crashed while loading. */
function Examples({ entry }: { entry: SignatureEntry }) {
    const { t, locale } = useI18n();
    const words = t.drops.signatures;
    return (
        <div className="flex flex-col gap-2 py-2">
            <p className="text-meta text-ink-3">{words.examplesTitle}</p>
            <ul className="flex flex-col gap-1.5">
                {entry.examples.map((example) => (
                    <li
                        key={example}
                        className="rounded-xs border border-line bg-surface px-2.5 py-1.5 font-mono text-small whitespace-pre-wrap text-ink [font-variant-ligatures:none] [overflow-wrap:anywhere]"
                    >
                        {example}
                    </li>
                ))}
            </ul>
            {entry.loadingShare !== null && (
                <p className="text-small text-ink-2">
                    {words.loadingShare(formatShare(locale, entry.loadingShare))}
                </p>
            )}
        </div>
    );
}

/** The crashes of each module or game build. */
function RollupTable({ rows, tab }: { rows: CrashRollup[]; tab: 'modules' | 'builds' }) {
    const { t, locale, format } = useI18n();
    const words = t.drops.signatures;
    const ranked = rows.toSorted((a, b) => b.drops - a.drops);
    const name = (row: CrashRollup) =>
        row.key === null ? (
            <span className="text-ink-3">{tab === 'modules' ? words.noModule : words.noBuild}</span>
        ) : (
            <span className={cn(table.mono, 'whitespace-normal [overflow-wrap:anywhere]')}>{row.key}</span>
        );
    return (
        <>
            <div className={cn(table.scroll, 'rounded-none max-md:hidden')}>
                <table className={table.table}>
                    <caption className="sr-only">
                        {tab === 'modules' ? words.modulesCaption : words.buildsCaption}
                    </caption>
                    <thead>
                        <tr>
                            <th scope="col" className={table.th}>
                                {tab === 'modules' ? words.module : words.build}
                            </th>
                            <th scope="col" className={cn(table.th, 'text-right')}>
                                {words.crashes}
                            </th>
                            <th scope="col" className={cn(table.th, 'w-[34%]')}>
                                <span className="sr-only">{words.share}</span>
                            </th>
                            <th scope="col" className={cn(table.th, 'text-right')}>
                                {words.signaturesCount}
                            </th>
                        </tr>
                    </thead>
                    <tbody>
                        {ranked.map((row, index) => (
                            <tr key={row.key ?? ''} className={table.row}>
                                <td className={table.td}>{name(row)}</td>
                                <td className={number}>{format.number(row.drops)}</td>
                                <td className={table.td}>
                                    <div className="flex items-center gap-2.5">
                                        <ShareBar share={row.share} first={index === 0} />
                                        <span className="w-10 text-right text-small">
                                            {formatShare(locale, row.share)}
                                        </span>
                                    </div>
                                </td>
                                <td className={cn(table.td, 'text-right')}>
                                    {format.number(row.signatures)}
                                </td>
                            </tr>
                        ))}
                    </tbody>
                </table>
            </div>
            <ul className="flex flex-col md:hidden">
                {ranked.map((row, index) => (
                    <li key={row.key ?? ''} className="flex flex-col gap-1.5 border-b border-line px-4 py-3">
                        <div className="flex items-baseline justify-between gap-3">
                            {name(row)}
                            <span className="text-small whitespace-nowrap text-ink">
                                {words.meta(row.drops)} · {formatShare(locale, row.share)}
                            </span>
                        </div>
                        <ShareBar share={row.share} first={index === 0} />
                    </li>
                ))}
            </ul>
        </>
    );
}

/**
 * Players who crashed again and again: counted for everyone, named for who sees players; those
 * without a confirmed identity are counted after the names.
 */
function RepeatCrashers({ repeat, since }: { repeat: RepeatCrasher[]; since: string | null }) {
    const { t } = useI18n();
    const words = t.drops.signatures;
    if (repeat.length === 0) {
        return null;
    }
    const named = repeat.flatMap(({ player, crashes }) => (player === null ? [] : [{ player, crashes }]));
    const others = repeat.length - named.length;
    return (
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 rounded-b-card bg-bg-subtle px-5 py-2.5 text-meta text-ink-3 max-md:px-4">
            {named.length === 0 ? (
                <span>{words.repeat(repeat.length)}</span>
            ) : (
                <span>
                    {words.repeatNamed(
                        named.map(({ player, crashes }, index) => (
                            <span key={player.id}>
                                {index > 0 && ' · '}
                                <Link
                                    to="/players/$id"
                                    params={{ id: String(player.id) }}
                                    className="font-medium text-ink-2 underline-offset-2 hover:underline"
                                >
                                    <PlayerName name={player.name ?? ''} />
                                </Link>{' '}
                                {words.repeatCrashes(crashes)}
                            </span>
                        )),
                        others,
                    )}
                </span>
            )}
            {since !== null && <span>{t.drops.playersSince(since)}</span>}
        </div>
    );
}
