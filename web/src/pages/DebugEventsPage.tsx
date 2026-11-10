import { keepPreviousData, useQuery } from '@tanstack/react-query';
import { ChevronRight } from 'lucide-react';
import { Fragment, useId, useState } from 'react';

import { debugEventsQuery } from '../api/debug';
import type { DebugEvents } from '../api/generated/DebugEvents';
import type { DebugWindow } from '../api/generated/DebugWindow';
import type { EventRow } from '../api/generated/EventRow';
import type { EventWay } from '../api/generated/EventWay';
import type { TrafficPoint } from '../api/generated/TrafficPoint';
import { Segmented } from '../components/Segmented';
import { table } from '../components/table';
import { DebugFrame } from '../debug/DebugFrame';
import { Find, Share, Tile, Tiles, useNumbers, useResources, useRoomy } from '../debug/parts';
import { Client, useOnline, type Online } from '../debug/players';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

const VIEWS = ['events', 'resources', 'players', 'refused'] as const;
type View = (typeof VIEWS)[number];

/** The ways that cross the network, whose bytes are the traffic with clients. */
const WITH_CLIENTS: ReadonlySet<EventWay> = new Set(['toClient', 'toAll', 'fromClients']);
/** The ways the server receives, whose handlers take time. */
const RECEIVED: ReadonlySet<EventWay> = new Set(['fromClients', 'local', 'fromEngine']);
/** Rows of the refused card, before "See them all". */
const REFUSED_SHOWN = 3;

const keyOf = (row: EventRow) => `${row.way} ${row.name}`;

/** Debug, Events: what each resource sends and receives, as its own resources count it. */
export function DebugEventsPage() {
    const { t } = useI18n();
    const words = t.debug.events;
    const [window, setWindow] = useState<DebugWindow>('5m');
    const [paused, setPaused] = useState(false);
    const events = useQuery({ ...debugEventsQuery(window, !paused), placeholderData: keepPreviousData });
    return (
        <DebugFrame
            title={words.title}
            lead={words.lead}
            query={events}
            window={window}
            onWindow={setWindow}
            paused={paused}
            onPause={setPaused}
        >
            {events.data !== undefined && <Events data={events.data} window={window} />}
        </DebugFrame>
    );
}

function Events({ data, window }: { data: DebugEvents; window: DebugWindow }) {
    const { t, format } = useI18n();
    const words = t.debug.events;
    const numbers = useNumbers(data.seconds);
    const online = useOnline();
    const [view, setView] = useState<View>('events');
    const [find, setFind] = useState('');
    const [picked, setPicked] = useState<string | null>(null);
    const [frame, roomy] = useRoomy();
    const { totals } = data;
    const span = t.debug.windows[window];

    return (
        <>
            <Tiles label={words.title}>
                <Tile
                    label={words.totals.toClients}
                    value={numbers.rate(totals.toClients.count)}
                    unit={t.debug.aSecond}
                    about={numbers.bytesRate(totals.toClients.bytes)}
                />
                <Tile
                    label={words.totals.fromClients}
                    value={numbers.rate(totals.fromClients.count)}
                    unit={t.debug.aSecond}
                    about={numbers.bytesRate(totals.fromClients.bytes)}
                />
                <Tile
                    label={words.totals.local}
                    value={numbers.rate(totals.local)}
                    unit={t.debug.aSecond}
                    about={words.totals.localAbout}
                />
                <Tile
                    label={words.totals.handlers}
                    value={numbers.ms(totals.handlerUs / data.seconds)}
                    unit={words.totals.handlersUnit}
                    about={words.totals.handlersAbout}
                />
                <Tile
                    label={words.totals.refused}
                    value={format.number(totals.refused)}
                    unit={words.totals.refusedIn(span)}
                    about={words.totals.refusedBy(totals.refusedPlayers)}
                    warning={totals.refused > 0}
                />
            </Tiles>
            <Traffic series={data.series} title={words.traffic(span)} />
            <div className="flex flex-wrap items-center gap-2.5">
                <Segmented
                    label={words.view}
                    choices={VIEWS}
                    value={view}
                    words={words.views}
                    onChange={setView}
                />
                {(view === 'events' || view === 'resources') && (
                    <Find label={words.find} value={find} onChange={setFind} />
                )}
            </div>
            <section ref={frame} className="card min-w-0" aria-label={words.views[view]}>
                {view === 'events' && (
                    <ByEvent
                        data={data}
                        find={find}
                        picked={picked}
                        onPick={setPicked}
                        online={online}
                        roomy={roomy}
                    />
                )}
                {view === 'resources' && <ByResource data={data} find={find} />}
                {view === 'players' && <ByPlayer data={data} online={online} />}
                {view === 'refused' && <Refused data={data} online={online} />}
            </section>
            {view !== 'refused' && (
                <RefusedCard data={data} span={span} online={online} onAll={() => setView('refused')} />
            )}
        </>
    );
}

/** The bytes a second to clients and from clients over the window, on one scale from zero. */
function Traffic({ series, title }: { series: TrafficPoint[]; title: string }) {
    const { t, format } = useI18n();
    const words = t.debug.events;
    const titleId = useId();
    const to = series.map((point) => point.toClients / point.seconds);
    const from = series.map((point) => point.fromClients / point.seconds);
    const top = Math.max(1, ...to, ...from);
    const [first, last] = [series[0], series.at(-1)];
    const path = (values: number[]) =>
        values
            .map((value, index) => {
                const x = values.length === 1 ? 300 : (index / (values.length - 1)) * 600;
                return `${index === 0 ? 'M' : 'L'}${x.toFixed(1)} ${(66 - (value / top) * 60).toFixed(1)}`;
            })
            .join('');
    const rate = (values: number[]) => t.debug.bytesRate(format.bytes(Math.round(values.at(-1) ?? 0)));

    return (
        <section className="card flex flex-col gap-2.5 px-5 py-4 max-md:px-4" aria-labelledby={titleId}>
            <div className="flex flex-wrap items-center gap-x-4 gap-y-1">
                <h2 id={titleId} className="text-small font-semibold text-ink">
                    {title}
                </h2>
                <span className="flex items-center gap-1.5 text-meta text-ink-2">
                    <span className="h-0.5 w-3.5 bg-accent" aria-hidden="true" />
                    {words.trafficTo}
                </span>
                <span className="flex items-center gap-1.5 text-meta text-ink-2">
                    <span className="w-3.5 border-t-2 border-dashed border-ink-3" aria-hidden="true" />
                    {words.trafficFrom}
                </span>
            </div>
            <svg
                className="block h-18 w-full overflow-visible"
                viewBox="0 0 600 72"
                preserveAspectRatio="none"
                {...{ role: 'img', 'aria-label': words.trafficLabel(rate(to), rate(from)) }}
            >
                <path
                    d="M0 66.5H600"
                    className="stroke-line"
                    strokeWidth={1}
                    vectorEffect="non-scaling-stroke"
                />
                <path
                    d={path(to)}
                    fill="none"
                    className="stroke-accent"
                    strokeWidth={1.5}
                    strokeLinejoin="round"
                    vectorEffect="non-scaling-stroke"
                />
                <path
                    d={path(from)}
                    fill="none"
                    className="stroke-ink-3"
                    strokeWidth={1.5}
                    strokeDasharray="4 4"
                    strokeLinejoin="round"
                    vectorEffect="non-scaling-stroke"
                />
            </svg>
            {first !== undefined && last !== undefined && (
                <div className="flex justify-between font-mono text-micro text-ink-3">
                    <span>{format.timestamp(first.at)}</span>
                    <span>{format.timestamp(last.at)}</span>
                </div>
            )}
        </section>
    );
}

/**
 * The events of the window. One opens by its name: what is known of it, and who sends it most, show
 * under its own row. Without room for every column, the resource goes under the event's name.
 */
function ByEvent({
    data,
    find,
    picked,
    onPick,
    online,
    roomy,
}: {
    data: DebugEvents;
    find: string;
    picked: string | null;
    onPick: (key: string | null) => void;
    online: Online;
    roomy: boolean;
}) {
    const { t } = useI18n();
    const words = t.debug.events;
    const numbers = useNumbers(data.seconds);
    const resourcesOf = useResources();
    const detailId = useId();
    const needle = find.trim().toLowerCase();
    const rows = data.events.filter(
        (row) => needle === '' || `${row.name} ${row.resources.join(' ')}`.toLowerCase().includes(needle),
    );
    const busiest = Math.max(
        1,
        ...data.events.filter((row) => WITH_CLIENTS.has(row.way)).map((row) => row.bytes),
    );
    if (rows.length === 0) {
        return (
            <p className="p-5 text-small text-ink-3 max-md:p-4">
                {data.events.length === 0 ? words.none : t.debug.noMatch}
            </p>
        );
    }
    return (
        <div className={cn(table.scroll, '@container')}>
            <table className={table.table}>
                <caption className="sr-only">{words.views.events}</caption>
                <thead>
                    <tr>
                        <th scope="col" className={cn(table.th, 'min-w-48')}>
                            {words.columns.event}
                        </th>
                        <th scope="col" className={table.th}>
                            {words.columns.way}
                        </th>
                        {roomy && (
                            <th scope="col" className={table.th}>
                                {words.columns.resource}
                            </th>
                        )}
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.rate}
                        </th>
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.sizes}
                        </th>
                        <th scope="col" className={cn(table.th, 'min-w-36')}>
                            {words.columns.clients}
                        </th>
                    </tr>
                </thead>
                <tbody>
                    {rows.map((row) => {
                        const key = keyOf(row);
                        const crosses = WITH_CLIENTS.has(row.way);
                        const open = key === picked;
                        const resources = resourcesOf(row.resources, row.resourceCount);
                        return (
                            <Fragment key={key}>
                                <tr className={cn(table.row, open && 'bg-surface-3')}>
                                    <td className={table.td}>
                                        <button
                                            type="button"
                                            aria-expanded={open}
                                            aria-controls={open ? detailId : undefined}
                                            className={cn(
                                                table.mono,
                                                'flex items-baseline gap-1.5 rounded-xs text-left whitespace-normal underline-offset-2 [overflow-wrap:anywhere] hover:underline',
                                            )}
                                            onClick={() => onPick(open ? null : key)}
                                        >
                                            <ChevronRight
                                                size={12}
                                                strokeWidth={2}
                                                className={cn(
                                                    'flex-none self-center text-ink-3 transition-transform',
                                                    open && 'rotate-90',
                                                )}
                                                aria-hidden="true"
                                            />
                                            {row.name}
                                        </button>
                                        {!roomy && (
                                            <span className="block pl-[18px] text-meta text-ink-3">
                                                {resources}
                                            </span>
                                        )}
                                    </td>
                                    <td className={cn(table.td, 'whitespace-nowrap')}>
                                        {words.ways[row.way]}
                                    </td>
                                    {roomy && <td className={table.td}>{resources}</td>}
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
                                        {crosses ? (
                                            <Share
                                                share={row.bytes / busiest}
                                                strong={row.way !== 'fromClients'}
                                            >
                                                {numbers.bytesRate(row.bytes)}
                                            </Share>
                                        ) : (
                                            <span className="font-mono text-meta text-ink-3">
                                                {words.staysHere}
                                            </span>
                                        )}
                                    </td>
                                </tr>
                                {open && (
                                    <tr id={detailId} className="[&:last-child>td]:border-b-0">
                                        <td
                                            colSpan={roomy ? 6 : 5}
                                            className="border-b border-line bg-surface-2 p-0"
                                        >
                                            {/* As wide as what shows of the table, and in sight while it scrolls. */}
                                            <div className="sticky left-0 grid w-[100cqw] max-w-3xl items-start gap-x-10 gap-y-3 px-5 py-4 max-md:px-4 sm:grid-cols-2">
                                                <Facts row={row} seconds={data.seconds} />
                                                <Senders row={row} seconds={data.seconds} online={online} />
                                            </div>
                                        </td>
                                    </tr>
                                )}
                            </Fragment>
                        );
                    })}
                </tbody>
            </table>
        </div>
    );
}

function ByResource({ data, find }: { data: DebugEvents; find: string }) {
    const { t } = useI18n();
    const words = t.debug.events;
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
    const flow = (count: number, bytes: number) =>
        count === 0 ? '0' : `${numbers.rate(count)} · ${numbers.bytesRate(bytes)}`;
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
                            {words.columns.toClients}
                        </th>
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.fromClients}
                        </th>
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.local}
                        </th>
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.handlers}
                        </th>
                    </tr>
                </thead>
                <tbody>
                    {rows.map((row) => (
                        <tr key={row.resource} className={table.row}>
                            <td className={cn(table.td, 'font-medium text-ink')}>{row.resource}</td>
                            <td className={cn(table.td, table.mono, 'text-right tabular-nums')}>
                                {flow(row.toClients.count, row.toClients.bytes)}
                            </td>
                            <td className={cn(table.td, table.mono, 'text-right tabular-nums')}>
                                {flow(row.fromClients.count, row.fromClients.bytes)}
                            </td>
                            <td className={cn(table.td, table.mono, 'text-right tabular-nums')}>
                                {numbers.rate(row.local)}
                            </td>
                            <td className={cn(table.td, table.mono, 'text-right tabular-nums')}>
                                {words.msASecond(numbers.ms(row.handlerUs / data.seconds))}
                            </td>
                        </tr>
                    ))}
                </tbody>
            </table>
        </div>
    );
}

function ByPlayer({ data, online }: { data: DebugEvents; online: Online }) {
    const { t } = useI18n();
    const words = t.debug.events;
    const numbers = useNumbers(data.seconds);
    if (data.players.length === 0) {
        return <p className="p-5 text-small text-ink-3 max-md:p-4">{words.none}</p>;
    }
    const busiest = Math.max(1, ...data.players.map((row) => row.count));
    return (
        <div className={table.scroll}>
            <table className={table.table}>
                <caption className="sr-only">{words.views.players}</caption>
                <thead>
                    <tr>
                        <th scope="col" className={table.th}>
                            {words.columns.player}
                        </th>
                        <th scope="col" className={cn(table.th, 'min-w-48')}>
                            {words.columns.sent}
                        </th>
                        <th scope="col" className={table.th}>
                            {words.columns.top}
                        </th>
                    </tr>
                </thead>
                <tbody>
                    {data.players.map((row) => (
                        <tr key={row.player} className={table.row}>
                            <td className={table.td}>
                                <Client netId={row.player} online={online} />
                            </td>
                            <td className={table.td}>
                                <Share share={row.count / busiest} strong={false}>
                                    {`${t.debug.rate(numbers.rate(row.count))} · ${numbers.bytesRate(row.bytes)}`}
                                </Share>
                            </td>
                            <td
                                className={cn(
                                    table.td,
                                    table.mono,
                                    'whitespace-normal [overflow-wrap:anywhere]',
                                )}
                            >
                                {row.top}
                            </td>
                        </tr>
                    ))}
                </tbody>
            </table>
        </div>
    );
}

function Refused({ data, online }: { data: DebugEvents; online: Online }) {
    const { t, format } = useI18n();
    const words = t.debug.events;
    if (data.refused.length === 0) {
        return <p className="p-5 text-small text-ink-3 max-md:p-4">{words.refusedCard.none}</p>;
    }
    return (
        <div className={table.scroll}>
            <table className={table.table}>
                <caption className="sr-only">{words.views.refused}</caption>
                <thead>
                    <tr>
                        <th scope="col" className={table.th}>
                            {words.columns.event}
                        </th>
                        <th scope="col" className={table.th}>
                            {words.columns.player}
                        </th>
                        <th scope="col" className={table.th}>
                            {words.columns.resource}
                        </th>
                        <th scope="col" className={cn(table.th, 'text-right')}>
                            {words.columns.times}
                        </th>
                    </tr>
                </thead>
                <tbody>
                    {data.refused.map((row) => (
                        <tr key={`${row.resource} ${row.name} ${row.player}`} className={table.row}>
                            <td
                                className={cn(
                                    table.td,
                                    table.mono,
                                    'whitespace-normal [overflow-wrap:anywhere]',
                                )}
                            >
                                {row.name}
                            </td>
                            <td className={table.td}>
                                <Client netId={row.player} online={online} />
                            </td>
                            <td className={table.td}>{row.resource}</td>
                            <td className={cn(table.td, table.mono, 'text-right tabular-nums')}>
                                {format.number(row.count)}
                            </td>
                        </tr>
                    ))}
                </tbody>
            </table>
        </div>
    );
}

/** An event's rate, its size, and the time its handlers take when the server receives it. */
function Facts({ row, seconds }: { row: EventRow; seconds: number }) {
    const { t } = useI18n();
    const words = t.debug.events.picked;
    const numbers = useNumbers(seconds);
    return (
        <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1.5 text-small">
            <dt className="text-ink-3">{words.rate}</dt>
            <dd className="text-right font-mono text-ink tabular-nums">
                {t.debug.rate(numbers.rate(row.count))}
            </dd>
            <dt className="text-ink-3">{words.size}</dt>
            <dd className="text-right font-mono text-ink">
                {words.sizeValue(numbers.each(row.bytes, row.count), numbers.size(row.largest))}
            </dd>
            {RECEIVED.has(row.way) && (
                <>
                    <dt className="text-ink-3">{words.handler}</dt>
                    <dd className="text-right font-mono text-ink tabular-nums">
                        {t.debug.events.msEach(numbers.ms(row.count === 0 ? 0 : row.handlerUs / row.count))}
                    </dd>
                </>
            )}
        </dl>
    );
}

/** The clients that send an event most, each with the way to their profile. */
function Senders({ row, seconds, online }: { row: EventRow; seconds: number; online: Online }) {
    const { t } = useI18n();
    const words = t.debug.events.picked;
    const numbers = useNumbers(seconds);
    if (row.senders.length === 0) {
        return null;
    }
    return (
        <div className="flex min-w-0 flex-col gap-2">
            <h3 className="font-mono text-micro tracking-[0.08em] text-ink-3 uppercase">{words.senders}</h3>
            <ul className="flex flex-col gap-1">
                {row.senders.map((sender) => (
                    <li
                        key={sender.player}
                        className="flex min-h-8 items-center justify-between gap-3 rounded-control bg-surface-3 px-2.5 py-1 text-small"
                    >
                        <Client netId={sender.player} online={online} />
                        <span className="flex-none font-mono text-meta text-ink tabular-nums">
                            {t.debug.rate(numbers.rate(sender.count))}
                        </span>
                    </li>
                ))}
            </ul>
            <p className="text-meta text-pretty text-ink-2">{words.sendersAbout}</p>
        </div>
    );
}

/** What clients sent that no resource opened to them: the first rows, and the way to them all. */
function RefusedCard({
    data,
    span,
    online,
    onAll,
}: {
    data: DebugEvents;
    span: string;
    online: Online;
    onAll: () => void;
}) {
    const { t, format } = useI18n();
    const words = t.debug.events.refusedCard;
    const titleId = useId();
    if (data.totals.refused === 0) {
        return null;
    }
    return (
        <section className="card flex flex-col gap-2.5 px-5 py-4 max-md:px-4" aria-labelledby={titleId}>
            <h2 id={titleId} className="flex items-center gap-2 text-small font-semibold text-ink">
                <span className="led led-warning" aria-hidden="true" />
                {words.title(data.totals.refused, span)}
            </h2>
            <p className="text-small text-pretty text-ink-2">{words.about}</p>
            <ul className="flex flex-col gap-1.5">
                {data.refused.slice(0, REFUSED_SHOWN).map((row) => (
                    <li
                        key={`${row.resource} ${row.name} ${row.player}`}
                        className="flex flex-wrap items-baseline gap-x-2.5 gap-y-0.5 text-small"
                    >
                        <span className="font-mono text-ink [overflow-wrap:anywhere]">{row.name}</span>
                        <Client netId={row.player} online={online} />
                        <span className="font-mono text-meta text-ink-3 tabular-nums">
                            × {format.number(row.count)}
                        </span>
                    </li>
                ))}
            </ul>
            {data.refused.length > REFUSED_SHOWN && (
                <button
                    type="button"
                    className="self-start rounded-xs text-small font-medium text-ink-2 underline underline-offset-2 hover:text-ink"
                    onClick={onAll}
                >
                    {words.all}
                </button>
            )}
        </section>
    );
}
