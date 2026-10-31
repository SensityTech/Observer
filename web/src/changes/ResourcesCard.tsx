import { Link } from '@tanstack/react-router';
import { Boxes, Lock, Search, TriangleAlert } from 'lucide-react';
import { useId, useState } from 'react';

import type { Comparison } from '../api/generated/Comparison';
import type { LiveChange } from '../api/generated/LiveChange';
import type { ResourceChange } from '../api/generated/ResourceChange';
import type { ResourceChangeKind } from '../api/generated/ResourceChangeKind';
import { Card } from '../components/Card';
import { Chip } from '../components/Chip';
import { inputClass } from '../components/Field';
import { table } from '../components/table';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { Toggle } from './controls';
import { escapeInvisible } from './invisible';
import { FILTERED, RESOURCE_CHIP, changedResources } from './words';

/** Live changes shown before "Show all". */
const SHORT_LIST = 8;

/** Whether a chip shows the resource: Checking counts those a partial snapshot left unknown too. */
const shows = (kind: ResourceChangeKind, resource: ResourceChange) =>
    resource.change === kind || (kind === 'checking' && resource.change === 'unknown');

/**
 * The resources that changed, filtered by kind and searched, then what started and stopped while
 * the server ran.
 */
export function ResourcesCard({ comparison }: { comparison: Comparison }) {
    const { t } = useI18n();
    const words = t.changes.resources;
    const id = useId();
    const [shown, setShown] = useState<ReadonlySet<ResourceChangeKind>>(new Set());
    const [query, setQuery] = useState('');
    const { counts, resources, to } = comparison;
    const kinds = FILTERED.filter((kind) => counts[kind] > 0);
    // A kind picked while the snapshot hashed may have settled into others since.
    const picked = kinds.filter((kind) => shown.has(kind));
    const needle = query.trim().toLowerCase();
    const visible = resources.filter(
        (resource) =>
            (picked.length === 0 || picked.some((kind) => shows(kind, resource))) &&
            (needle === '' ||
                resource.name.toLowerCase().includes(needle) ||
                resource.path.toLowerCase().includes(needle)),
    );
    const toggle = (kind: ResourceChangeKind, pressed: boolean) => {
        const next = new Set<ResourceChangeKind>(picked);
        if (pressed) {
            next.add(kind);
        } else {
            next.delete(kind);
        }
        setShown(next);
    };

    let body;
    if (to.baseline) {
        body = <p className="px-6 py-6 text-center text-ui text-ink-3">{words.baseline}</p>;
    } else if (resources.length === 0) {
        body = <p className="px-6 py-6 text-center text-ui text-ink-3">{words.none}</p>;
    } else {
        body = (
            <>
                <div className="flex flex-wrap items-center gap-2 border-b border-line px-5 py-3 max-md:px-4">
                    <div className="relative min-w-48 flex-[0_1_16rem] max-md:flex-1">
                        <Search
                            size={14}
                            strokeWidth={1.5}
                            className="pointer-events-none absolute top-1/2 left-2.75 -translate-y-1/2 text-ink-3"
                            aria-hidden="true"
                        />
                        <input
                            type="search"
                            aria-label={words.search}
                            placeholder={words.search}
                            className={cn(inputClass, 'h-8 pl-8.5 text-ui')}
                            value={query}
                            onChange={(event) => setQuery(event.target.value)}
                        />
                    </div>
                    {kinds.length > 1 && (
                        <fieldset className="flex flex-wrap gap-1.5">
                            <legend className="sr-only">{words.filters}</legend>
                            {kinds.map((kind) => (
                                <Toggle
                                    key={kind}
                                    pressed={picked.includes(kind)}
                                    onChange={(pressed) => toggle(kind, pressed)}
                                >
                                    {words.kinds[kind]} <span className="tabular-nums">{counts[kind]}</span>
                                </Toggle>
                            ))}
                        </fieldset>
                    )}
                </div>
                {visible.length === 0 ? (
                    <p className="px-6 py-6 text-center text-ui text-ink-3">{words.noMatch}</p>
                ) : (
                    <div className={cn(table.scroll, comparison.live.length > 0 && 'rounded-none')}>
                        <table className={table.table}>
                            <caption className="sr-only">{words.caption}</caption>
                            <thead>
                                <tr>
                                    {[
                                        words.resource,
                                        words.change,
                                        words.version,
                                        words.files,
                                        words.lines,
                                        words.by,
                                    ].map((heading) => (
                                        <th key={heading} scope="col" className={table.th}>
                                            {heading}
                                        </th>
                                    ))}
                                </tr>
                            </thead>
                            <tbody>
                                {visible.map((resource) => (
                                    <ResourceRow
                                        key={`${resource.path} ${resource.change}`}
                                        comparison={comparison}
                                        resource={resource}
                                    />
                                ))}
                            </tbody>
                        </table>
                    </div>
                )}
            </>
        );
    }

    return (
        <Card
            id={`${id}-resources`}
            icon={Boxes}
            title={words.title}
            meta={to.baseline ? undefined : words.meta(changedResources(counts))}
            flush
        >
            {body}
            {comparison.live.length > 0 && <LiveChanges changes={comparison.live} />}
        </Card>
    );
}

function ResourceRow({ comparison, resource }: { comparison: Comparison; resource: ResourceChange }) {
    const { t, format } = useI18n();
    const words = t.changes.resources;
    const chip = RESOURCE_CHIP[resource.change];
    const warning = chip.tone === 'warning';
    const version =
        resource.fromVersion === null ||
        resource.toVersion === null ||
        resource.fromVersion === resource.toVersion
            ? escapeInvisible(resource.toVersion ?? resource.fromVersion ?? '')
            : t.changes.fromTo(escapeInvisible(resource.fromVersion), escapeInvisible(resource.toVersion));
    const files = resource.files;
    const sub = cn(table.sub, warning && 'text-ink-2');
    return (
        <tr className={warning ? 'bg-warning-soft [&:last-child>td]:border-b-0' : table.row}>
            <td
                className={cn(
                    table.td,
                    'min-w-56 align-top',
                    warning && 'shadow-[inset_2px_0_0_var(--warning-fill)]',
                )}
            >
                <Link
                    to="/changes/resource"
                    search={{
                        from: comparison.from.id,
                        // The next check of the disk deletes this one: the link follows the latest.
                        to: comparison.to.kind === 'check' ? 'disk' : comparison.to.id,
                        path: resource.path,
                    }}
                    className={cn(table.mono, 'font-medium underline-offset-2 hover:underline')}
                >
                    {escapeInvisible(resource.name)}
                </Link>
                <div className={sub}>
                    <span className="font-mono [overflow-wrap:anywhere]">
                        {escapeInvisible(resource.path)}
                    </span>
                    {resource.fromPath !== null && (
                        <span className="font-mono [overflow-wrap:anywhere]">
                            {words.fromPath(escapeInvisible(resource.fromPath))}
                        </span>
                    )}
                    {resource.escrowed && <Chip icon={Lock}>{words.escrowed}</Chip>}
                    {resource.duplicate && (
                        <Chip tone="warning" icon={TriangleAlert}>
                            {words.duplicate}
                        </Chip>
                    )}
                </div>
            </td>
            <td className={cn(table.td, 'align-top')}>
                <Chip tone={chip.tone} icon={chip.icon}>
                    {words.kinds[resource.change]}
                </Chip>
                {resource.changed && (resource.change === 'renamed' || resource.change === 'moved') && (
                    <div className={sub}>{words.filesToo}</div>
                )}
            </td>
            <td className={cn(table.td, table.mono, 'align-top')}>{version}</td>
            <td className={cn(table.td, table.mono, 'align-top text-ink-2')}>
                {files.added + files.modified + files.removed === 0
                    ? ''
                    : t.changes.files(
                          format.number(files.added),
                          format.number(files.modified),
                          format.number(files.removed),
                      )}
            </td>
            <td className={cn(table.td, table.mono, 'align-top text-ink-2')}>
                {resource.lines === null
                    ? ''
                    : t.changes.lines(
                          format.number(resource.lines.added),
                          format.number(resource.lines.removed),
                      )}
            </td>
            <td className={cn(table.td, 'align-top')}>{resource.by.join(', ')}</td>
        </tr>
    );
}

function LiveChanges({ changes }: { changes: LiveChange[] }) {
    const { t, format } = useI18n();
    const words = t.changes.live;
    const [all, setAll] = useState(false);
    const shown = all ? changes : changes.slice(0, SHORT_LIST);
    return (
        <div className="border-t border-line">
            <h3 className="px-5 pt-4 pb-2 text-ui font-semibold text-ink max-md:px-4">{words.title}</h3>
            <ul aria-label={words.caption} className="flex flex-col gap-1.5 px-5 pb-4 max-md:px-4">
                {shown.map((change) => (
                    <li
                        key={`${change.at} ${change.name} ${change.change}`}
                        className="flex flex-wrap items-center gap-x-2 gap-y-1 text-ui text-ink-2"
                    >
                        <time dateTime={change.at} className="font-mono text-small text-ink-3">
                            {format.timestamp(change.at)}
                        </time>
                        <Chip>{words.kinds[change.change]}</Chip>
                        <span className="font-mono text-small text-ink">{escapeInvisible(change.name)}</span>
                        <span>{change.by === null ? words.byUnknown : words.by(change.by)}</span>
                        {change.command !== null && (
                            <code className="code-inline">{escapeInvisible(change.command)}</code>
                        )}
                    </li>
                ))}
            </ul>
            {changes.length > SHORT_LIST && (
                <div className="px-5 pb-4 max-md:px-4">
                    <button
                        type="button"
                        className="text-small font-medium text-ink-2 underline-offset-2 hover:text-ink hover:underline max-md:min-h-11"
                        aria-expanded={all}
                        onClick={() => setAll(!all)}
                    >
                        {all ? words.showFewer : words.showAll(changes.length)}
                    </button>
                </div>
            )}
        </div>
    );
}
