import { useVirtualizer } from '@tanstack/react-virtual';
import { Lock } from 'lucide-react';
import { useRef } from 'react';

import type { DiffLine } from '../api/generated/DiffLine';
import type { DiffLineKind } from '../api/generated/DiffLineKind';
import type { Segment } from '../api/generated/Segment';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { hunkHeader, type DiffRow } from './diffRows';

/** Both versions' line numbers, or one column: on a phone, and in a whole file. */
export type Numbering = 'both' | 'one';

const LINE_HEIGHT = 18;

const TINT = {
    added: 'bg-diff-add-bg',
    removed: 'bg-diff-del-bg',
    context: '',
} satisfies Record<DiffLineKind, string>;

const WORD = {
    added: 'bg-diff-add-word',
    removed: 'bg-diff-del-word',
    context: '',
} satisfies Record<DiffLineKind, string>;

const SIGN = {
    added: { glyph: '+', ink: 'text-term-green' },
    removed: { glyph: '−', ink: 'text-term-red' },
    context: { glyph: ' ', ink: '' },
} satisfies Record<DiffLineKind, { glyph: string; ink: string }>;

/**
 * Lines of code on the console's ground, virtualised as the console is: a diff of thousands of lines
 * renders the rows in view. Lines keep their own height when they wrap.
 */
export function CodeLines({
    rows,
    numbering,
    wrap,
    label,
    onMore,
}: {
    rows: DiffRow[];
    numbering: Numbering;
    wrap: boolean;
    label: string;
    /** Asks for more unchanged lines around each change, from a hunk's header. */
    onMore?: () => void;
}) {
    const scroller = useRef<HTMLElement>(null);
    // The virtualizer renders the rows it measures again on its own; nothing here is memoised around it.
    // oxlint-disable-next-line react/incompatible-library
    const virtualizer = useVirtualizer({
        count: rows.length,
        getScrollElement: () => scroller.current,
        estimateSize: () => LINE_HEIGHT,
        overscan: 20,
        getItemKey: (index) => rows[index]?.key ?? index,
    });
    const width = wrap ? 'w-full' : 'w-max min-w-full';
    return (
        <section
            ref={scroller}
            aria-label={label}
            // oxlint-disable-next-line jsx-a11y/no-noninteractive-tabindex -- the keyboard scrolls the lines once it can focus them
            tabIndex={0}
            className="relative max-h-[70dvh] min-h-0 overflow-auto overscroll-contain bg-console py-1 font-mono text-meta text-term-fg scrollbar-thin [font-variant-ligatures:none] focus-visible:outline-offset-[-2px]"
        >
            <div className={cn('relative', width)} style={{ height: virtualizer.getTotalSize() }}>
                {virtualizer.getVirtualItems().map((item) => {
                    const row = rows[item.index];
                    if (row === undefined) {
                        return null;
                    }
                    return (
                        <div
                            key={item.key}
                            ref={virtualizer.measureElement}
                            data-index={item.index}
                            className={cn('absolute top-0 left-0', width)}
                            style={{ transform: `translateY(${item.start}px)` }}
                        >
                            <Row row={row} numbering={numbering} wrap={wrap} onMore={onMore} />
                        </div>
                    );
                })}
            </div>
        </section>
    );
}

function Row({
    row,
    numbering,
    wrap,
    onMore,
}: {
    row: DiffRow;
    numbering: Numbering;
    wrap: boolean;
    onMore?: () => void;
}) {
    const { t } = useI18n();
    if (row.type === 'hunk') {
        return (
            <div className="flex min-h-7 flex-wrap items-center gap-x-3 bg-surface-2 px-3 text-term-dim">
                <span className="whitespace-pre select-none">{hunkHeader(row.hunk)}</span>
                {onMore !== undefined && (
                    <button
                        type="button"
                        className="font-sans font-medium text-ink-2 underline-offset-2 hover:text-ink hover:underline max-md:min-h-11"
                        onClick={onMore}
                    >
                        {t.changes.diff.moreLines}
                    </button>
                )}
            </div>
        );
    }
    if (row.type === 'pair') {
        // Side by side, lines always wrap: a long line would run under the other version.
        return (
            <div className="grid grid-cols-2">
                <Line line={row.before} numbers={{ from: row.before?.from ?? null }} wrap />
                <Line line={row.after} numbers={{ to: row.after?.to ?? null }} wrap />
            </div>
        );
    }
    const { line } = row;
    return (
        <Line
            line={line}
            numbers={numbering === 'both' ? { from: line.from, to: line.to } : { to: line.to ?? line.from }}
            wrap={wrap}
        />
    );
}

function Line({
    line,
    numbers,
    wrap,
}: {
    line: DiffLine | null;
    numbers: { from?: number | null; to?: number | null };
    wrap: boolean;
}) {
    const { t } = useI18n();
    const kind = line?.kind ?? 'context';
    const sign = SIGN[kind];
    return (
        <div className="flex min-h-4.5 min-w-0">
            <span className="sticky left-0 flex shrink-0 bg-console text-ink-3 tabular-nums select-none">
                {'from' in numbers && <span className="w-12 pr-2 text-right">{numbers.from ?? ''}</span>}
                {'to' in numbers && <span className="w-12 pr-2 text-right">{numbers.to ?? ''}</span>}
            </span>
            <span className={cn('flex min-w-0 flex-1', line !== null && TINT[kind])}>
                <span className={cn('w-5 shrink-0 text-center select-none', sign.ink)}>
                    <span aria-hidden="true">{sign.glyph}</span>
                    {kind !== 'context' && (
                        <span className="sr-only">
                            {kind === 'added' ? t.changes.diff.added : t.changes.diff.removed}
                        </span>
                    )}
                </span>
                <span
                    className={cn(
                        'min-w-0 flex-1 pr-4',
                        wrap ? 'whitespace-pre-wrap [overflow-wrap:anywhere]' : 'whitespace-pre',
                    )}
                >
                    {line !== null && <Segments segments={line.segments} kind={kind} />}
                </span>
            </span>
        </div>
    );
}

/** A line's pieces: the words that changed against its other version stand out, and a secret is a chip. */
function Segments({ segments, kind }: { segments: Segment[]; kind: DiffLineKind }) {
    const { t } = useI18n();
    // Each piece keyed by where it starts in the line, which no two share.
    const placed = segments.reduce<{ segment: Segment; at: number }[]>((list, segment) => {
        const last = list.at(-1);
        return [...list, { segment, at: last === undefined ? 0 : last.at + last.segment.text.length }];
    }, []);
    return placed.map(({ segment, at }) => {
        if (segment.redacted) {
            return (
                <span key={at} className="whitespace-nowrap">
                    <span className="code-inline inline-flex items-center gap-1 align-baseline">
                        <Lock size={11} strokeWidth={2} aria-hidden="true" />
                        <span className="sr-only">{t.changes.diff.hiddenSecret}</span>
                        {segment.text}
                    </span>
                    {segment.changed && (
                        <span className="ml-1.5 font-sans text-term-yellow">
                            {t.changes.diff.secretChanged}
                        </span>
                    )}
                </span>
            );
        }
        return segment.changed && kind !== 'context' ? (
            <span key={at} className={cn('rounded-xs', WORD[kind])}>
                {segment.text}
            </span>
        ) : (
            <span key={at}>{segment.text}</span>
        );
    });
}
