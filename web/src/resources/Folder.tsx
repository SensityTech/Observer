import { ChevronDown, Ellipsis, LoaderCircle, Play, RotateCw, Square, TriangleAlert } from 'lucide-react';

import type { ResourceRow } from '../api/generated/ResourceRow';
import type { ResourceState } from '../api/generated/ResourceState';
import type { ResourceVerb } from '../api/resources';
import { Button, buttonVariants } from '../components/Button';
import { chipVariants } from '../components/Chip';
import { Menu } from '../components/Menu';
import { Toggletip } from '../components/Toggletip';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { didNotStart, type Group } from './list';
import { changeLine, warningLines, type Order } from './words';

const LEDS = {
    started: 'led-ok',
    starting: 'led-pending',
    stopped: 'led-off',
    stopping: 'led-pending',
    unknown: 'led-off',
} satisfies Record<ResourceState, string>;

const ICONS = { start: Play, stop: Square, restart: RotateCw } satisfies Record<ResourceVerb, typeof Play>;

type Ask = (verb: ResourceVerb, row: ResourceRow) => void;

type RowProps = {
    row: ResourceRow;
    now: number;
    /** The order FXServer has not answered on it: no other is taken meanwhile. */
    order: Order | null;
    /** Null for an account that reads the list without starting or stopping anything. */
    onAsk: Ask | null;
};

/**
 * The orders a resource takes as it stands: a start while it is down, a restart or a stop while it
 * is up. A restart under way keeps its place on a resource FXServer has stopped and not yet started.
 */
const verbsOf = (row: ResourceRow): ResourceVerb[] =>
    row.state === 'started' ? ['restart', 'stop'] : ['start'];

/** Which of a row's buttons waits: the one asked for, or the only one left once the state moved. */
const waits = (verb: ResourceVerb, verbs: ResourceVerb[], order: Order | null) =>
    order !== null && (order.verb === verb || !verbs.includes(order.verb));

function Warnings({ row }: { row: ResourceRow }) {
    const { t } = useI18n();
    const words = t.resources;
    if (row.warnings === null) {
        return null;
    }
    return (
        <Toggletip
            className={cn(chipVariants({ tone: 'warning' }), 'flex-none')}
            tip={
                <>
                    <span className="text-ink">{words.warned(row.warnings.count)}</span>
                    {warningLines(t, row.warnings).map((line) => (
                        <span key={line}>{line}</span>
                    ))}
                </>
            }
        >
            <TriangleAlert size={12} strokeWidth={2} aria-hidden="true" />
            {words.warnings(row.warnings.count)}
        </Toggletip>
    );
}

function Row({ row, now, order, onAsk }: RowProps) {
    const { t, format } = useI18n();
    const words = t.resources;
    const failed = didNotStart(row);
    const change = changeLine(t, format, row, now, order);
    const verbs = verbsOf(row);
    return (
        <li
            className={cn(
                'relative border-b border-line px-5 last:border-b-0 hover:bg-surface-2 max-md:flex max-md:flex-col max-md:gap-1 max-md:py-2.5 max-md:pr-14 max-md:pl-4',
                'md:grid md:min-h-11 md:items-center md:gap-x-4 md:py-1.5',
                onAsk === null
                    ? 'md:grid-cols-[minmax(0,1fr)_5rem_14.5rem]'
                    : 'md:grid-cols-[minmax(0,1fr)_5rem_14.5rem_10.75rem]',
            )}
        >
            <div className="flex min-w-0 items-center gap-2">
                <span className="truncate font-mono text-small text-ink [font-variant-ligatures:none]">
                    {row.name}
                </span>
                <Warnings row={row} />
            </div>
            <span className="truncate font-mono text-meta text-ink-3 max-md:hidden">{row.version ?? ''}</span>
            <div className="flex min-w-0 flex-col max-md:flex-row max-md:flex-wrap max-md:items-center max-md:gap-x-2">
                <span
                    className={cn(
                        'flex items-center gap-2 text-ui font-medium',
                        failed ? 'text-danger-ink' : 'text-ink',
                    )}
                >
                    <span className={cn('led', failed ? 'led-down' : LEDS[row.state])} aria-hidden="true" />
                    {failed ? words.didNotStart : words.states[row.state]}
                </span>
                {change !== null && (
                    <span className="truncate pl-4 text-meta text-ink-3 max-md:pl-0" title={change}>
                        {change}
                    </span>
                )}
                {row.version !== null && (
                    <span className="truncate font-mono text-meta text-ink-3 md:hidden">v{row.version}</span>
                )}
            </div>
            {onAsk !== null && !row.controllable && (
                <div className="flex justify-end max-md:justify-start">
                    <Toggletip
                        className="text-meta text-ink-3 underline decoration-dotted underline-offset-2"
                        tip={words.consoleOnlyTip}
                    >
                        {words.consoleOnly}
                    </Toggletip>
                </div>
            )}
            {onAsk !== null && row.controllable && (
                <>
                    <div className="flex justify-end gap-1 max-md:hidden">
                        {verbs.map((verb) => (
                            <Button
                                key={verb}
                                size="sm"
                                tone="ghost"
                                icon={ICONS[verb]}
                                pending={waits(verb, verbs, order)}
                                disabled={order !== null}
                                onClick={() => onAsk(verb, row)}
                            >
                                {words.verbs[verb]}
                            </Button>
                        ))}
                    </div>
                    <div className="md:hidden max-md:absolute max-md:top-1.5 max-md:right-2">
                        {order === null ? (
                            <Menu
                                label={words.actionsOn(row.name)}
                                buttonClassName={buttonVariants({ size: 'icon-sm', tone: 'ghost' })}
                                placement="below-end"
                                items={verbs.map((verb) => ({
                                    label: words.verbs[verb],
                                    icon: ICONS[verb],
                                    onSelect: () => onAsk(verb, row),
                                }))}
                            >
                                <Ellipsis size={16} strokeWidth={1.5} aria-hidden="true" />
                            </Menu>
                        ) : (
                            <span className="grid size-11 place-items-center text-ink-3">
                                <LoaderCircle
                                    size={16}
                                    strokeWidth={1.5}
                                    className="animate-spin"
                                    aria-hidden="true"
                                />
                            </span>
                        )}
                    </div>
                </>
            )}
        </li>
    );
}

type FolderProps = {
    group: Group;
    now: number;
    folded: boolean;
    onFold: (folded: boolean) => void;
    orderOf: (row: ResourceRow) => Order | null;
    onAsk: Ask | null;
};

/**
 * The resources of one folder, under a line that folds them and counts those started among all it
 * holds, whatever the filters show.
 */
export function Folder({ group, now, folded, onFold, orderOf, onAsk }: FolderProps) {
    const { t } = useI18n();
    const words = t.resources;
    return (
        <section className="card overflow-hidden">
            <h2>
                <button
                    type="button"
                    aria-expanded={!folded}
                    className={cn(
                        'flex min-h-11 w-full items-center gap-2.5 px-5 text-left focus-visible:-outline-offset-2 max-md:px-4',
                        !folded && 'border-b border-line',
                    )}
                    onClick={() => onFold(!folded)}
                >
                    <ChevronDown
                        size={14}
                        strokeWidth={2}
                        className={cn('flex-none text-ink-3 transition-transform', folded && '-rotate-90')}
                        aria-hidden="true"
                    />
                    <span className="truncate font-mono text-small font-medium text-ink [font-variant-ligatures:none]">
                        {group.folder}
                    </span>
                    {group.origin === 'build' && (
                        <span className="flex-none text-meta text-ink-3 max-md:hidden">
                            {words.folder.build}
                        </span>
                    )}
                    <span className="ml-auto flex-none text-meta text-ink-3">
                        {words.folder.started(group.started, group.total)}
                    </span>
                </button>
            </h2>
            {!folded && (
                <ul>
                    {group.rows.map((row) => (
                        <Row key={row.name} row={row} now={now} order={orderOf(row)} onAsk={onAsk} />
                    ))}
                </ul>
            )}
        </section>
    );
}
