import { useQuery } from '@tanstack/react-query';
import { Link } from '@tanstack/react-router';
import { Search } from 'lucide-react';
import { useId, useState } from 'react';

import type { HeavyAsset } from '../api/generated/HeavyAsset';
import type { MissingFile } from '../api/generated/MissingFile';
import { startWarningsQuery } from '../api/server';
import { Chip } from '../components/Chip';
import { inputClass } from '../components/Field';
import { Notice } from '../components/Notice';
import { Switch } from '../components/Switch';
import { table } from '../components/table';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';

/** Rows shown before "Show more". */
const PAGE = 200;

type Kind = 'heavy' | 'missing';

/**
 * What FXServer warned about as it last started its resources, as a list to work from: the heavy
 * streamed files, or the files manifests name that their resources do not hold.
 */
export function ToolWarningsPage({ kind }: { kind: Kind }) {
    const { t, format } = useI18n();
    const hub = t.tools.hub;
    const words = hub[kind];
    const warnings = useQuery(startWarningsQuery);
    const titleId = useId();
    const [query, setQuery] = useState('');
    const [oversized, setOversized] = useState(false);
    const [limit, setLimit] = useState(PAGE);
    const said = warnings.data ?? null;
    const needle = query.trim().toLowerCase();
    const matches = (row: { resource: string; file: string }) =>
        needle === '' || `${row.resource}/${row.file}`.toLowerCase().includes(needle);

    const heavy: HeavyAsset[] =
        kind !== 'heavy' || said === null
            ? []
            : said.heavy
                  .filter((asset) => matches(asset) && (!oversized || asset.oversized))
                  .toSorted((left, right) => right.tenths - left.tenths);
    const missing: MissingFile[] =
        kind !== 'missing' || said === null
            ? []
            : said.missing
                  .filter(matches)
                  .toSorted(
                      (left, right) =>
                          left.resource.localeCompare(right.resource) || left.file.localeCompare(right.file),
                  );
    const total = said === null ? 0 : said[kind].length;
    const found = kind === 'heavy' ? heavy.length : missing.length;

    return (
        <div className="flex flex-col gap-5 max-md:gap-4">
            <nav aria-label={hub.breadcrumb} className="flex items-center gap-2 text-small text-ink-3">
                <Link
                    to="/tools"
                    className="font-medium text-ink-2 underline-offset-2 hover:text-ink hover:underline"
                >
                    {t.tools.title}
                </Link>
                <span aria-hidden="true">/</span>
                <span>{words.title}</span>
            </nav>
            <div className="page-head">
                <h1 className="page-title">{words.title}</h1>
            </div>
            <p className="-mt-2 max-w-3xl text-body text-ink-2">{words.long}</p>
            {warnings.isError && <Notice tone="danger">{errorMessage(t, warnings.error)}</Notice>}
            {said !== null && said.startedAt === null && <Notice>{hub.noStartLong}</Notice>}
            {said !== null && said.startedAt !== null && (
                <section className="card" aria-labelledby={titleId}>
                    <div className="card-head">
                        <h2 id={titleId} className="card-title">
                            {words.count(total)}
                        </h2>
                        <span className="text-small text-ink-3 tabular-nums">
                            {hub.fromStart(format.timestamp(said.startedAt))}
                        </span>
                        <div className="ml-auto flex flex-wrap items-center gap-x-4 gap-y-2">
                            {kind === 'heavy' && (
                                <label className="flex items-center gap-2 text-small text-ink-2">
                                    <Switch
                                        checked={oversized}
                                        onChange={(event) => setOversized(event.target.checked)}
                                    />
                                    {hub.heavy.onlyOversized}
                                </label>
                            )}
                            <div className="relative">
                                <Search
                                    size={14}
                                    strokeWidth={1.5}
                                    className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-ink-3"
                                    aria-hidden="true"
                                />
                                <input
                                    type="search"
                                    aria-label={hub.search}
                                    placeholder={hub.search}
                                    value={query}
                                    className={cn(inputClass, 'h-8 w-56 pl-8 text-small max-md:h-11')}
                                    onChange={(event) => {
                                        setQuery(event.target.value);
                                        setLimit(PAGE);
                                    }}
                                />
                            </div>
                        </div>
                    </div>
                    {found === 0 ? (
                        <p className="p-5 text-small text-ink-3 max-md:p-4">
                            {total === 0 ? words.none : hub.noMatch}
                        </p>
                    ) : (
                        <div className="overflow-x-auto">
                            {kind === 'heavy' ? (
                                <table className={table.table}>
                                    <thead>
                                        <tr>
                                            <th scope="col" className={table.th}>
                                                {hub.columns.resource}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {hub.columns.file}
                                            </th>
                                            <th scope="col" className={cn(table.th, 'text-right')}>
                                                {hub.columns.size}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {hub.columns.memory}
                                            </th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {heavy.slice(0, limit).map((asset) => (
                                            <tr
                                                key={`${asset.resource}/${asset.file}/${asset.memory}`}
                                                className={table.row}
                                            >
                                                <td className={cn(table.td, 'font-mono text-small')}>
                                                    {asset.resource}
                                                </td>
                                                <td
                                                    className={cn(
                                                        table.td,
                                                        'font-mono text-small [overflow-wrap:anywhere]',
                                                    )}
                                                >
                                                    {asset.file}
                                                </td>
                                                <td className={cn(table.td, 'text-right tabular-nums')}>
                                                    {hub.heavy.size(
                                                        format.number(asset.tenths / 10, {
                                                            minimumFractionDigits: 1,
                                                            maximumFractionDigits: 1,
                                                        }),
                                                    )}
                                                </td>
                                                <td className={table.td}>
                                                    <span className="flex flex-wrap items-center gap-2">
                                                        {hub.heavy.memory[asset.memory]}
                                                        {asset.oversized && (
                                                            <Chip tone="warning">{hub.heavy.oversized}</Chip>
                                                        )}
                                                    </span>
                                                </td>
                                            </tr>
                                        ))}
                                    </tbody>
                                </table>
                            ) : (
                                <table className={table.table}>
                                    <thead>
                                        <tr>
                                            <th scope="col" className={table.th}>
                                                {hub.columns.resource}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {hub.columns.file}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {hub.columns.asked}
                                            </th>
                                            <th scope="col" className={table.th}>
                                                {hub.columns.named}
                                            </th>
                                        </tr>
                                    </thead>
                                    <tbody>
                                        {missing.slice(0, limit).map((file) => (
                                            <tr
                                                key={`${file.resource}/${file.file}/${file.kind}/${file.line}`}
                                                className={table.row}
                                            >
                                                <td className={cn(table.td, 'font-mono text-small')}>
                                                    {file.resource}
                                                </td>
                                                <td
                                                    className={cn(
                                                        table.td,
                                                        'font-mono text-small [overflow-wrap:anywhere]',
                                                    )}
                                                >
                                                    {file.file}
                                                </td>
                                                <td className={cn(table.td, 'font-mono text-small')}>
                                                    {file.kind}
                                                </td>
                                                <td
                                                    className={cn(
                                                        table.td,
                                                        'font-mono text-small tabular-nums',
                                                    )}
                                                >
                                                    {hub.missing.named(
                                                        file.manifest,
                                                        format.number(file.line, { useGrouping: false }),
                                                    )}
                                                </td>
                                            </tr>
                                        ))}
                                    </tbody>
                                </table>
                            )}
                        </div>
                    )}
                    {(found > limit || said.truncated) && (
                        <div className="flex flex-wrap items-center gap-3 border-t border-line px-5 py-3 text-small text-ink-3 max-md:px-4">
                            {found > limit && (
                                <button
                                    type="button"
                                    className="font-medium text-ink underline-offset-2 hover:underline max-md:min-h-11"
                                    onClick={() => setLimit(limit + PAGE)}
                                >
                                    {hub.more(found - limit)}
                                </button>
                            )}
                            {said.truncated && <span>{hub.truncated}</span>}
                        </div>
                    )}
                </section>
            )}
            <p className="max-w-3xl text-small text-ink-3">{words.next}</p>
        </div>
    );
}
