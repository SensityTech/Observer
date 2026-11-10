import { keepPreviousData, useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { useId, useState } from 'react';

import { debugHttpQuery, knowHost } from '../api/debug';
import type { DebugHttp } from '../api/generated/DebugHttp';
import type { DebugWindow } from '../api/generated/DebugWindow';
import type { HttpRow } from '../api/generated/HttpRow';
import { Permission } from '../api/generated/Permission';
import { allows, useMe } from '../auth/me';
import { Button, buttonVariants } from '../components/Button';
import { Notice } from '../components/Notice';
import { Segmented } from '../components/Segmented';
import { table } from '../components/table';
import { DebugFrame } from '../debug/DebugFrame';
import { Find, Tile, Tiles, useNumbers } from '../debug/parts';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { useNow } from '../lib/useNow';

const VIEWS = ['all', 'fresh'] as const;
type View = (typeof VIEWS)[number];

/** Debug, HTTP requests: which resource calls which host, and the hosts never called before. */
export function DebugHttpPage() {
    const { t } = useI18n();
    const words = t.debug.http;
    const [window, setWindow] = useState<DebugWindow>('15m');
    const [paused, setPaused] = useState(false);
    const http = useQuery({ ...debugHttpQuery(window, !paused), placeholderData: keepPreviousData });
    return (
        <DebugFrame
            title={words.title}
            lead={words.lead}
            query={http}
            window={window}
            onWindow={setWindow}
            paused={paused}
            onPause={setPaused}
        >
            {http.data !== undefined && <Requests data={http.data} />}
        </DebugFrame>
    );
}

function Requests({ data }: { data: DebugHttp }) {
    const { t, format } = useI18n();
    const words = t.debug.http;
    const numbers = useNumbers(data.seconds);
    const [view, setView] = useState<View>('all');
    const [find, setFind] = useState('');
    const { totals } = data;
    const needle = find.trim().toLowerCase();
    const rows = data.requests.filter(
        (row) =>
            (view === 'all' || row.new) &&
            (needle === '' || `${row.host} ${row.resource}`.toLowerCase().includes(needle)),
    );
    const fresh = data.requests.filter((row) => row.new);
    const now = useNow(60_000);
    const learning = data.learningUntil !== null && Date.parse(data.learningUntil) > now;

    return (
        <>
            <Tiles label={words.title}>
                <Tile
                    label={words.totals.requests}
                    value={numbers.aMinute(totals.requests)}
                    unit={t.debug.aMinute}
                    about={words.totals.requestsBy(totals.resources)}
                />
                <Tile
                    label={words.totals.sent}
                    value={format.bytes(Math.round((totals.bytes / data.seconds) * 60))}
                    unit={t.debug.aMinute}
                    about={words.totals.sentAbout}
                />
                <Tile
                    label={words.totals.hosts}
                    value={format.number(totals.hosts)}
                    unit={words.totals.hostsUnit}
                    about={words.totals.hostsNew(totals.newHosts)}
                    warning={totals.newHosts > 0}
                />
                <Tile
                    label={words.totals.resources}
                    value={format.number(totals.resources)}
                    unit={words.totals.resourcesUnit}
                    about={words.totals.resourcesOf(data.status.reporting)}
                />
            </Tiles>
            {learning && data.learningUntil !== null && (
                <Notice>{words.learning(format.timestamp(data.learningUntil, { seconds: false }))}</Notice>
            )}
            <div className="flex flex-wrap items-center gap-2.5">
                <Segmented
                    label={words.view}
                    choices={VIEWS}
                    value={view}
                    words={{ all: words.views.all(totals.hosts), fresh: words.views.fresh(totals.newHosts) }}
                    onChange={setView}
                />
                <Find label={words.find} value={find} onChange={setFind} />
            </div>
            {/* What asks for the owner's eye, before the list it comes from. */}
            <Fresh rows={fresh} />
            <section className="card min-w-0" aria-label={words.title}>
                {rows.length === 0 ? (
                    <p className="p-5 text-small text-ink-3 max-md:p-4">
                        {data.requests.length === 0
                            ? words.none
                            : needle === ''
                              ? words.noneNew
                              : t.debug.noMatch}
                    </p>
                ) : (
                    <Hosts rows={rows} seconds={data.seconds} />
                )}
            </section>
            <section className="card flex flex-col gap-2 px-5 py-4 max-md:px-4">
                <h2 className="text-small font-semibold text-ink">{words.kept}</h2>
                <p className="text-small text-pretty text-ink-2">{words.keptAbout}</p>
            </section>
        </>
    );
}

function Hosts({ rows, seconds }: { rows: HttpRow[]; seconds: number }) {
    const { t, format } = useI18n();
    const words = t.debug.http;
    const numbers = useNumbers(seconds);
    return (
        <div className={table.scroll}>
            <table className={table.table}>
                <caption className="sr-only">{words.title}</caption>
                <thead>
                    <tr>
                        <th scope="col" className={cn(table.th, 'min-w-44')}>
                            {words.columns.host}
                        </th>
                        <th scope="col" className={table.th}>
                            {words.columns.method}
                        </th>
                        <th scope="col" className={table.th}>
                            {words.columns.resource}
                        </th>
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.rate}
                        </th>
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.sent}
                        </th>
                        <th scope="col" className={table.th}>
                            {words.columns.first}
                        </th>
                    </tr>
                </thead>
                <tbody>
                    {rows.map((row) => {
                        const first =
                            row.firstCalled === null
                                ? ''
                                : format.timestamp(row.firstCalled, { seconds: false });
                        return (
                            <tr
                                key={`${row.resource} ${row.method} ${row.host}`}
                                className={cn(table.row, row.new && 'bg-surface-3')}
                            >
                                <td
                                    className={cn(
                                        table.td,
                                        table.mono,
                                        'whitespace-normal [overflow-wrap:anywhere]',
                                    )}
                                >
                                    {row.host}
                                </td>
                                <td className={cn(table.td, 'font-mono text-meta')}>{row.method}</td>
                                <td className={table.td}>{row.resource}</td>
                                <td className={cn(table.td, table.mono, 'text-right tabular-nums')}>
                                    {numbers.aMinute(row.count)}
                                </td>
                                <td
                                    className={cn(
                                        table.td,
                                        'text-right font-mono text-small whitespace-nowrap',
                                    )}
                                >
                                    {row.bytes === 0 ? (
                                        <span className="text-ink-3">{words.noBody}</span>
                                    ) : (
                                        numbers.each(row.bytes, row.count)
                                    )}
                                </td>
                                <td className={cn(table.td, 'whitespace-nowrap')}>
                                    {row.new ? (
                                        <span className="inline-flex items-center gap-2 text-ink">
                                            <span className="led led-warning" aria-hidden="true" />
                                            {words.isNew(first)}
                                        </span>
                                    ) : (
                                        first
                                    )}
                                </td>
                            </tr>
                        );
                    })}
                </tbody>
            </table>
        </div>
    );
}

/** The hosts no resource had called before, each with what the owner can do about it. */
function Fresh({ rows }: { rows: HttpRow[] }) {
    const { t, format } = useI18n();
    const words = t.debug.http.fresh;
    const me = useMe();
    const client = useQueryClient();
    const titleId = useId();
    const know = useMutation({
        mutationFn: knowHost,
        onSuccess: () => client.invalidateQueries({ queryKey: ['debug', 'http'] }),
    });
    // One line a host and a resource, whatever the methods it was called with.
    const pairs = rows.filter(
        (row, index) =>
            rows.findIndex((other) => other.resource === row.resource && other.host === row.host) === index,
    );
    if (pairs.length === 0) {
        return null;
    }
    const seesChanges = allows(me, Permission.ChangesView);
    return (
        <section className="card flex flex-col gap-3 px-5 py-4 max-md:px-4" aria-labelledby={titleId}>
            <h2 id={titleId} className="flex items-center gap-2 text-small font-semibold text-ink">
                <span className="led led-warning" aria-hidden="true" />
                {words.title(pairs.length)}
            </h2>
            {know.isError && (
                <Notice tone="danger">
                    {words.failed} {errorMessage(t, know.error)}
                </Notice>
            )}
            <ul className="flex flex-col gap-3.5">
                {pairs.map((row) => (
                    <li key={`${row.resource} ${row.host}`} className="flex flex-col gap-2">
                        <p className="text-small text-pretty text-ink-2 [overflow-wrap:anywhere]">
                            {words.about(
                                row.resource,
                                row.host,
                                row.firstCalled === null
                                    ? ''
                                    : format.timestamp(row.firstCalled, { seconds: false }),
                            )}
                        </p>
                        <div className="flex flex-wrap gap-2">
                            {me.isOwner && (
                                <Button
                                    tone="ink"
                                    size="sm"
                                    aria-label={words.knowNamed(row.host)}
                                    pending={
                                        know.isPending &&
                                        know.variables.host === row.host &&
                                        know.variables.resource === row.resource
                                    }
                                    disabled={know.isPending}
                                    onClick={() => know.mutate({ resource: row.resource, host: row.host })}
                                >
                                    {words.know}
                                </Button>
                            )}
                            {seesChanges && (
                                <Link to="/changes" className={buttonVariants({ size: 'sm' })}>
                                    {words.changes}
                                </Link>
                            )}
                        </div>
                    </li>
                ))}
            </ul>
        </section>
    );
}
