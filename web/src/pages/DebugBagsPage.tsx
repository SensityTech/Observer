import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { useId, useState } from 'react';

import { debugBagsQuery } from '../api/debug';
import type { DebugBags } from '../api/generated/DebugBags';
import type { DebugWindow } from '../api/generated/DebugWindow';
import { Segmented } from '../components/Segmented';
import { table } from '../components/table';
import { DebugFrame } from '../debug/DebugFrame';
import { Find, Share, Tile, Tiles, useNumbers, useResources, useRoomy } from '../debug/parts';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

const VIEWS = ['keys', 'resources'] as const;
type View = (typeof VIEWS)[number];

/** Debug, State bags: what resources write into state bags, and how much goes out to clients. */
export function DebugBagsPage() {
    const { t } = useI18n();
    const words = t.debug.bags;
    const [window, setWindow] = useState<DebugWindow>('5m');
    const [paused, setPaused] = useState(false);
    const bags = useQuery({ ...debugBagsQuery(window, !paused), placeholderData: keepPreviousData });
    return (
        <DebugFrame
            title={words.title}
            lead={words.lead}
            query={bags}
            window={window}
            onWindow={setWindow}
            paused={paused}
            onPause={setPaused}
        >
            {bags.data !== undefined && <Bags data={bags.data} window={window} />}
        </DebugFrame>
    );
}

function Bags({ data, window }: { data: DebugBags; window: DebugWindow }) {
    const { t, format } = useI18n();
    const words = t.debug.bags;
    const numbers = useNumbers(data.seconds);
    const [view, setView] = useState<View>('keys');
    const [find, setFind] = useState('');
    const [frame, roomy] = useRoomy();
    const { totals } = data;
    const share = totals.bytes === 0 ? 0 : totals.replicatedBytes / totals.bytes;

    return (
        <>
            <Tiles label={words.title}>
                <Tile
                    label={words.totals.writes}
                    value={numbers.rate(totals.writes)}
                    unit={t.debug.aSecond}
                    about={words.totals.writesBy(totals.resources)}
                />
                <Tile
                    label={words.totals.written}
                    value={format.bytes(Math.round(totals.bytes / data.seconds))}
                    unit={t.debug.aSecond}
                    about={words.totals.writtenAbout}
                />
                <Tile
                    label={words.totals.sent}
                    value={format.bytes(Math.round(totals.replicatedBytes / data.seconds))}
                    unit={t.debug.aSecond}
                    about={words.totals.sentShare(format.number(share, { style: 'percent' }))}
                />
                <Tile
                    label={words.totals.keys}
                    value={format.number(totals.keys)}
                    unit={words.totals.keysUnit}
                    about={words.totals.keysIn(t.debug.windows[window])}
                />
            </Tiles>
            <div className="flex flex-wrap items-center gap-2.5">
                <Segmented
                    label={words.view}
                    choices={VIEWS}
                    value={view}
                    words={words.views}
                    onChange={setView}
                />
                <Find label={words.find} value={find} onChange={setFind} />
            </div>
            <section ref={frame} className="card min-w-0" aria-label={words.views[view]}>
                {view === 'keys' ? (
                    <ByKey data={data} find={find} roomy={roomy} />
                ) : (
                    <ByResource data={data} find={find} />
                )}
            </section>
            <div className="grid items-start gap-3 sm:grid-cols-2">
                <Largest data={data} />
                <section className="card flex flex-col gap-2 px-5 py-4 only:sm:col-span-2 max-md:px-4">
                    <h2 className="text-small font-semibold text-ink">{words.reading}</h2>
                    <p className="text-small text-pretty text-ink-2">{words.readingAbout}</p>
                </section>
            </div>
        </>
    );
}

/** The keys written in the window. Without room for every column, who writes one goes under it. */
function ByKey({ data, find, roomy }: { data: DebugBags; find: string; roomy: boolean }) {
    const { t, format } = useI18n();
    const words = t.debug.bags;
    const numbers = useNumbers(data.seconds);
    const resourcesOf = useResources();
    const needle = find.trim().toLowerCase();
    const rows = data.keys.filter(
        (row) => needle === '' || `${row.key} ${row.resources.join(' ')}`.toLowerCase().includes(needle),
    );
    if (rows.length === 0) {
        return (
            <p className="p-5 text-small text-ink-3 max-md:p-4">
                {data.keys.length === 0 ? words.none : t.debug.noMatch}
            </p>
        );
    }
    return (
        <div className={table.scroll}>
            <table className={table.table}>
                <caption className="sr-only">{words.views.keys}</caption>
                <thead>
                    <tr>
                        <th scope="col" className={cn(table.th, 'min-w-40')}>
                            {words.columns.key}
                        </th>
                        <th scope="col" className={table.th}>
                            {words.columns.bag}
                        </th>
                        {roomy && (
                            <th scope="col" className={table.th}>
                                {words.columns.by}
                            </th>
                        )}
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.rate}
                        </th>
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.sizes}
                        </th>
                        <th scope="col" className={cn(table.th, 'min-w-40')}>
                            {words.columns.sent}
                        </th>
                    </tr>
                </thead>
                <tbody>
                    {rows.map((row) => {
                        const sent = row.count === 0 ? 0 : row.replicated / row.count;
                        const by = resourcesOf(row.resources, row.resourceCount);
                        return (
                            <tr key={`${row.kind} ${row.key}`} className={table.row}>
                                <td className={table.td}>
                                    <span
                                        className={cn(
                                            table.mono,
                                            'block whitespace-normal [overflow-wrap:anywhere]',
                                        )}
                                    >
                                        {row.key}
                                    </span>
                                    {!roomy && <span className="block text-meta text-ink-3">{by}</span>}
                                </td>
                                <td className={cn(table.td, 'whitespace-nowrap')}>{words.kinds[row.kind]}</td>
                                {roomy && <td className={table.td}>{by}</td>}
                                <td className={cn(table.td, table.mono, 'text-right tabular-nums')}>
                                    {numbers.rate(row.count)}
                                </td>
                                <td
                                    className={cn(
                                        table.td,
                                        'text-right font-mono text-small whitespace-nowrap',
                                    )}
                                >
                                    {t.debug.sizes(
                                        numbers.each(row.bytes, row.count),
                                        numbers.size(row.largest),
                                    )}
                                </td>
                                <td className={table.td}>
                                    {sent === 0 ? (
                                        <span className="font-mono text-meta text-ink-3">
                                            {words.staysHere}
                                        </span>
                                    ) : (
                                        <Share share={sent}>
                                            {sent >= 1
                                                ? words.every
                                                : words.some(format.number(sent, { style: 'percent' }))}
                                        </Share>
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

function ByResource({ data, find }: { data: DebugBags; find: string }) {
    const { t, format } = useI18n();
    const words = t.debug.bags;
    const numbers = useNumbers(data.seconds);
    const needle = find.trim().toLowerCase();
    const rows = data.resources.filter((row) => needle === '' || row.resource.toLowerCase().includes(needle));
    if (rows.length === 0) {
        return (
            <p className="p-5 text-small text-ink-3 max-md:p-4">
                {data.resources.length === 0 ? words.none : t.debug.noMatch}
            </p>
        );
    }
    return (
        <div className={table.scroll}>
            <table className={table.table}>
                <caption className="sr-only">{words.views.resources}</caption>
                <thead>
                    <tr>
                        <th scope="col" className={table.th}>
                            {words.columns.resource}
                        </th>
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.rate}
                        </th>
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.written}
                        </th>
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.sent}
                        </th>
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.keys}
                        </th>
                    </tr>
                </thead>
                <tbody>
                    {rows.map((row) => (
                        <tr key={row.resource} className={table.row}>
                            <td className={cn(table.td, 'font-medium text-ink')}>{row.resource}</td>
                            <td className={cn(table.td, table.mono, 'text-right tabular-nums')}>
                                {numbers.rate(row.count)}
                            </td>
                            <td className={cn(table.td, table.mono, 'text-right tabular-nums')}>
                                {numbers.bytesRate(row.bytes)}
                            </td>
                            <td className={cn(table.td, table.mono, 'text-right tabular-nums')}>
                                {numbers.bytesRate(row.replicatedBytes)}
                            </td>
                            <td className={cn(table.td, table.mono, 'text-right tabular-nums')}>
                                {format.number(row.keys)}
                            </td>
                        </tr>
                    ))}
                </tbody>
            </table>
        </div>
    );
}

function Largest({ data }: { data: DebugBags }) {
    const { t, format } = useI18n();
    const words = t.debug.bags;
    const titleId = useId();
    if (data.largest.length === 0) {
        return null;
    }
    return (
        <section className="card flex flex-col gap-2.5 px-5 py-4 max-md:px-4" aria-labelledby={titleId}>
            <h2 id={titleId} className="text-small font-semibold text-ink">
                {words.largest}
            </h2>
            <ul className="flex flex-col gap-2.5">
                {data.largest.map((write) => (
                    <li key={`${write.resource} ${write.bag} ${write.key}`} className="flex flex-col gap-0.5">
                        <span className="flex items-baseline justify-between gap-3 text-small">
                            <span className="min-w-0 font-mono text-ink [overflow-wrap:anywhere]">
                                {words.largestOf(write.bag, write.key)}
                            </span>
                            <span className="flex-none font-mono text-ink tabular-nums">
                                {format.bytes(write.bytes)}
                            </span>
                        </span>
                        <span className="text-meta text-ink-3">
                            {write.resource} · {write.replicated ? words.toClients : words.staysHere}
                        </span>
                    </li>
                ))}
            </ul>
        </section>
    );
}
