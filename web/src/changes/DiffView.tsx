import { keepPreviousData, useQuery } from '@tanstack/react-query';
import {
    Check,
    ChevronDown,
    ChevronLeft,
    ChevronRight,
    ChevronUp,
    Copy,
    FileText,
    GitCompare,
} from 'lucide-react';
import { useMemo, useRef, useState, type ReactNode } from 'react';

import { DEFAULT_CONTEXT, diffQuery, fileQuery } from '../api/changes';
import { ApiError } from '../api/client';
import type { FileChange } from '../api/generated/FileChange';
import { Button } from '../components/Button';
import { Chip } from '../components/Chip';
import { Notice } from '../components/Notice';
import { Code } from '../diagnostics/words';
import { errorMessage } from '../i18n/errors';
import { useI18n } from '../i18n/I18nProvider';
import { stringParam } from '../i18n/params';
import { PHONE, WIDE, useMedia } from '../lib/useMedia';
import { CodeLines } from './CodeLines';
import { Segmented, Toggle } from './controls';
import { splitRows, textRows, unifiedRows } from './diffRows';

/** Asked to be read more lines at a time, each time "Show 20 more lines" is pressed. */
const MORE_CONTEXT = 20;

type View = 'diff' | 'before' | 'after';

/**
 * A file between two snapshots: its lines when both versions are kept text, what can be said of it
 * otherwise, and the whole masked file of either side on request.
 */
export function DiffView({
    from,
    to,
    path,
    file,
    onPrevious,
    onNext,
}: {
    from: number;
    to: number;
    path: string;
    /** The file as its resource lists it; a file of the cfg chain is known from its diff alone. */
    file: FileChange | null;
    onPrevious?: () => void;
    onNext?: () => void;
}) {
    const { t, format } = useI18n();
    const words = t.changes.diff;
    const wide = useMedia(WIDE);
    const phone = useMedia(PHONE);
    const [layout, setLayout] = useState<'unified' | 'split'>('unified');
    const [ignoreWhitespace, setIgnoreWhitespace] = useState(false);
    const [wrap, setWrap] = useState(phone);
    const [context, setContext] = useState(DEFAULT_CONTEXT);
    const [page, setPage] = useState(0);
    const [view, setView] = useState<View>('diff');
    const [filePage, setFilePage] = useState(0);

    const readable = file === null || (file.kind === 'text' && file.kept && file.change !== 'unchanged');
    const diff = useQuery({
        ...diffQuery({ from, to, path, page, context, ignoreWhitespace }),
        enabled: readable && view === 'diff',
        placeholderData: keepPreviousData,
    });
    const shown = diff.data?.file ?? file;
    const sides: View[] =
        shown === null || shown.kind !== 'text'
            ? []
            : (['before', 'after'] as const).filter((side) =>
                  side === 'before' ? shown.change !== 'added' : shown.change !== 'removed',
              );
    const text = useQuery({
        ...fileQuery(
            view === 'before' ? from : to,
            view === 'before' ? (shown?.fromPath ?? path) : path,
            filePage,
        ),
        enabled: view !== 'diff',
        placeholderData: keepPreviousData,
    });
    const split = wide && layout === 'split';
    const lines = diff.data?.diff ?? null;
    const counts = lines ?? shown?.lines ?? null;
    const rows = useMemo(() => {
        if (view !== 'diff') {
            return text.data === undefined ? [] : textRows(text.data.text);
        }
        if (lines === null) {
            return [];
        }
        return split ? splitRows(lines.hunks) : unifiedRows(lines.hunks);
    }, [view, text.data, lines, split]);

    const showFile = (side: View) => {
        setView(side);
        setFilePage(0);
    };

    let body: ReactNode = null;
    if (view !== 'diff') {
        body = text.isError ? (
            <BodyNotice>
                <Notice tone="danger">{errorMessage(t, text.error)}</Notice>
            </BodyNotice>
        ) : text.data === undefined ? (
            <Waiting />
        ) : (
            <>
                <CodeLines rows={rows} numbering="one" wrap={wrap} label={words.lines(path)} />
                <Pages
                    page={text.data.text.page}
                    pages={text.data.text.pages}
                    busy={text.isPlaceholderData}
                    onPage={setFilePage}
                />
            </>
        );
    } else if (readable) {
        if (diff.isError) {
            body = (
                <BodyNotice>
                    <DiffRefusal error={diff.error} />
                </BodyNotice>
            );
        } else if (diff.data === undefined) {
            body = <Waiting />;
        } else if (lines !== null) {
            body = (
                <>
                    {lines.lineEndingsOnly && (
                        <BodyNotice>
                            <Notice>{words.lineEndings}</Notice>
                        </BodyNotice>
                    )}
                    {lines.hunks.length === 0 ? (
                        !lines.lineEndingsOnly && (
                            <p className="px-6 py-4 text-center text-ui text-ink-3">{words.noLine}</p>
                        )
                    ) : (
                        <CodeLines
                            rows={rows}
                            numbering={phone ? 'one' : 'both'}
                            wrap={wrap || split}
                            label={words.lines(path)}
                            onMore={() => setContext((current) => current + MORE_CONTEXT)}
                        />
                    )}
                    <Pages
                        page={lines.page}
                        pages={lines.pages}
                        busy={diff.isPlaceholderData}
                        onPage={setPage}
                    />
                </>
            );
        }
    }

    return (
        <section className="card" aria-label={words.label(path)}>
            <div className="sticky top-0 z-(--z-dock) flex flex-col gap-2.5 rounded-t-card border-b border-line bg-surface px-4 py-3 max-xl:top-(--mobilebar-height)">
                <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
                    <h3 className="min-w-0 font-mono text-small text-ink [overflow-wrap:anywhere] [font-variant-ligatures:none]">
                        {path}
                    </h3>
                    {shown !== null && <Chip>{words.fileKinds[shown.kind]}</Chip>}
                    {counts !== null && (
                        <span className="font-mono text-small whitespace-nowrap text-ink-2 [font-variant-ligatures:none]">
                            {t.changes.lines(format.number(counts.added), format.number(counts.removed))}
                        </span>
                    )}
                </div>
                <div className="flex flex-wrap items-center gap-2">
                    {view === 'diff' && readable && wide && (
                        <Segmented
                            label={words.layout}
                            value={layout}
                            options={[
                                ['unified', words.unified],
                                ['split', words.split],
                            ]}
                            onChange={setLayout}
                        />
                    )}
                    {view === 'diff' && readable && (
                        <Toggle
                            pressed={ignoreWhitespace}
                            onChange={(pressed) => {
                                setIgnoreWhitespace(pressed);
                                setPage(0);
                            }}
                        >
                            {words.ignoreWhitespace}
                        </Toggle>
                    )}
                    {!(view === 'diff' && split) && (
                        <Toggle pressed={wrap} onChange={setWrap}>
                            {words.wrap}
                        </Toggle>
                    )}
                    {view !== 'diff' && sides.length > 1 && (
                        <Segmented
                            label={words.side}
                            value={view}
                            options={[
                                ['before', words.before],
                                ['after', words.after],
                            ]}
                            onChange={showFile}
                        />
                    )}
                    {sides.length > 0 &&
                        (view === 'diff' ? (
                            <Button
                                size="sm"
                                icon={FileText}
                                onClick={() => showFile(sides.at(-1) ?? 'after')}
                            >
                                {words.viewFile}
                            </Button>
                        ) : (
                            readable && (
                                <Button size="sm" icon={GitCompare} onClick={() => setView('diff')}>
                                    {words.viewDiff}
                                </Button>
                            )
                        ))}
                    {(onPrevious !== undefined || onNext !== undefined) && (
                        <span className="ml-auto flex gap-1">
                            <Button
                                size="icon-sm"
                                tone="ghost"
                                icon={phone ? ChevronLeft : ChevronUp}
                                aria-label={words.previous}
                                disabled={onPrevious === undefined}
                                onClick={onPrevious}
                            />
                            <Button
                                size="icon-sm"
                                tone="ghost"
                                icon={phone ? ChevronRight : ChevronDown}
                                aria-label={words.next}
                                disabled={onNext === undefined}
                                onClick={onNext}
                            />
                        </span>
                    )}
                </div>
            </div>
            {shown !== null && view === 'diff' && <FileNotices file={shown} />}
            {body}
        </section>
    );
}

function BodyNotice({ children }: { children: ReactNode }) {
    return <div className="px-4 pt-3 last:pb-4">{children}</div>;
}

function Waiting() {
    const { t } = useI18n();
    return (
        <div className="flex flex-col gap-2 p-4" aria-busy="true" aria-label={t.changes.diff.loading}>
            {[0, 1, 2, 3].map((line) => (
                <div key={line} className="skeleton w-full" />
            ))}
        </div>
    );
}

/** Why the lines cannot be read, as the API refused them. */
function DiffRefusal({ error }: { error: Error }) {
    const { t } = useI18n();
    if (error instanceof ApiError && error.code === 'content_hidden') {
        return (
            <Notice>
                {t.changes.diff.hidden} <Code>{stringParam(error.params, 'glob')}</Code>
            </Notice>
        );
    }
    if (error instanceof ApiError && error.code === 'content_not_kept') {
        return <Notice>{t.changes.diff.notKept}</Notice>;
    }
    return <Notice tone="danger">{errorMessage(t, error)}</Notice>;
}

function Pages({
    page,
    pages,
    busy,
    onPage,
}: {
    page: number;
    pages: number;
    busy: boolean;
    onPage: (page: number) => void;
}) {
    const { t } = useI18n();
    if (pages <= 1) {
        return null;
    }
    return (
        <div
            className="flex flex-wrap items-center gap-2 rounded-b-card border-t border-line bg-bg-subtle py-2 pr-3 pl-4 text-meta text-ink-3"
            aria-busy={busy}
        >
            <output className="mr-auto">{t.changes.diff.page(page + 1, pages)}</output>
            <Button
                size="icon-sm"
                tone="ghost"
                icon={ChevronLeft}
                aria-label={t.changes.diff.previousPage}
                disabled={page === 0}
                onClick={() => onPage(page - 1)}
            />
            <Button
                size="icon-sm"
                tone="ghost"
                icon={ChevronRight}
                aria-label={t.changes.diff.nextPage}
                disabled={page + 1 >= pages}
                onClick={() => onPage(page + 1)}
            />
        </div>
    );
}

/** What can be said of a file besides its lines: its kind, its sizes and hashes, and what happened to it. */
function FileNotices({ file }: { file: FileChange }) {
    const { t, format } = useI18n();
    const words = t.changes.diff;
    const notices: { key: string; tone?: 'warning'; body: ReactNode }[] = [];
    const sizes = [file.fromSize, file.toSize]
        .filter((size) => size !== null)
        .map((size) => format.bytes(size));
    const sizeWords =
        sizes.length === 2 ? t.changes.fromTo(sizes[0] ?? '', sizes[1] ?? '') : (sizes[0] ?? '');
    const sameSize =
        file.change === 'modified' && file.fromSize === file.toSize && file.fromSha256 !== file.toSha256;
    if (file.kind === 'binary') {
        notices.push({
            key: 'binary',
            body: [words.binary, sizeWords, sameSize ? words.sameSize : null]
                .filter((part) => part !== null)
                .join(' · '),
        });
    } else if (file.kind === 'encrypted') {
        notices.push({
            key: 'encrypted',
            body: [file.change === 'unchanged' ? words.encrypted : words.encryptedChanged, sizeWords].join(
                ' · ',
            ),
        });
    } else if (file.kind === 'generated') {
        notices.push({ key: 'generated', body: `${words.generated} ${sizeWords}` });
    } else if (file.kind === 'hidden') {
        notices.push({
            key: 'hidden',
            body: (
                <>
                    {words.hidden} <Code>{file.hiddenBy ?? ''}</Code>
                </>
            ),
        });
    } else if (
        file.kind === 'archive' ||
        file.kind === 'offline' ||
        file.kind === 'link' ||
        file.kind === 'unreadable'
    ) {
        notices.push({ key: file.kind, body: words[file.kind] });
    } else if (file.change === 'unchanged') {
        notices.push({ key: 'unchanged', body: words.unchanged });
    } else if (!file.kept) {
        notices.push({ key: 'notKept', body: words.notKept });
    }
    for (const flag of file.flags) {
        notices.push({
            key: flag,
            tone: flag === 'reloadedLive' ? undefined : 'warning',
            body: words.flags[flag],
        });
    }
    if (file.writtenBy !== null && file.writtenAt !== null) {
        notices.push({
            key: 'written',
            body: t.changes.resource.writtenBy(
                file.writtenBy,
                format.timestamp(file.writtenAt, { seconds: false }),
            ),
        });
    }
    const hashes = [
        [words.shaBefore, file.fromSha256],
        [words.shaAfter, file.toSha256],
    ].filter((hash): hash is [string, string] => hash[1] !== null);
    if (notices.length === 0 && hashes.length === 0) {
        return null;
    }
    return (
        <div className="flex flex-col gap-2 px-4 pt-3 last:pb-4">
            {notices.map(({ key, tone, body }) => (
                <Notice key={key} tone={tone}>
                    {body}
                </Notice>
            ))}
            {hashes.length > 0 && (
                <dl className="flex flex-col gap-1">
                    {hashes.map(([label, hash]) => (
                        <div key={label} className="flex flex-wrap items-center gap-x-2.5">
                            <dt className="text-meta text-ink-3">{label}</dt>
                            <dd className="flex min-w-0 items-center gap-1">
                                <HashValue label={label} hash={hash} />
                            </dd>
                        </div>
                    ))}
                </dl>
            )}
        </div>
    );
}

/** A hash to check a vendor's release against, copied in one click where the browser allows it. */
function HashValue({ label, hash }: { label: string; hash: string }) {
    const { t } = useI18n();
    const value = useRef<HTMLElement>(null);
    const [copied, setCopied] = useState(false);
    const copy = async () => {
        try {
            await navigator.clipboard.writeText(hash);
            setCopied(true);
        } catch {
            // No clipboard outside a secure context: the hash is selected for the keyboard to copy.
            if (value.current !== null) {
                window.getSelection()?.selectAllChildren(value.current);
            }
        }
    };
    return (
        <>
            <code ref={value} className="font-mono text-small break-all text-ink">
                {hash}
            </code>
            <Button
                size="icon-sm"
                tone="ghost"
                icon={copied ? Check : Copy}
                aria-label={`${copied ? t.common.copied : t.common.copy}: ${label}`}
                onClick={() => void copy()}
            />
        </>
    );
}
