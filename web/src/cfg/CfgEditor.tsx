import { Search } from 'lucide-react';
import { useEffect, useMemo, useRef, useState } from 'react';

import type { CfgSpan } from '../api/generated/CfgSpan';
import type { Severity } from '../api/generated/Severity';
import { inputClass } from '../components/Field';
import { useI18n } from '../i18n/I18nProvider';
import { cn } from '../lib/cn';
import { TOKEN_CLASS } from '../setup/CfgBrowser';
import { lineTokens } from '../setup/cfgSyntax';

const LINE_HEIGHT = 20;
const SECRET = /<<secret-\d+>>/gu;

/** The text and the colours over it must lay out alike, character for character. */
const TYPE =
    'm-0 border-0 p-0 pr-16 pl-3 font-mono text-[12.5px] leading-5 tracking-normal whitespace-pre [font-variant-ligatures:none] [tab-size:4]';

const GUTTER = {
    blocking: 'text-danger-ink font-semibold',
    warning: 'text-warning font-semibold',
    note: 'text-accent-ink',
} satisfies Record<Severity, string>;

/** Scrolls the lines so that `line` shows a third of the way down. */
const reveal = (view: HTMLElement | null, line: number) => {
    if (view !== null) {
        view.scrollTop = Math.max(0, (line - 1) * LINE_HEIGHT - view.clientHeight / 3);
    }
};

/** A line as the preview of the setup colours it, a secret's mark kept as the text field holds it. */
const coloured = (line: string) => {
    const marks = line.match(SECRET) ?? [];
    const spans: CfgSpan[] = [];
    line.split(SECRET).forEach((piece, index) => {
        if (piece !== '') {
            spans.push({ text: piece, secret: false });
        }
        if (index < marks.length) {
            spans.push({ text: '', secret: true });
        }
    });
    let mark = 0;
    let at = 0;
    return lineTokens(spans).map((token) => {
        const text = token.kind === 'secret' ? (marks[mark++] ?? '') : token.text;
        const key = at;
        at += text.length;
        return { key, kind: token.kind, text };
    });
};

/**
 * The cfg as text: a plain text field, for the keyboard, the clipboard and assistive technology
 * to work as they do anywhere, under the same text coloured. `marks` flags the lines the checks
 * name; `jump` puts the caret on a line and brings it into view.
 */
export function CfgEditor({
    label,
    text,
    marks,
    jump,
    onChange,
}: {
    label: string;
    text: string;
    marks: ReadonlyMap<number, Severity>;
    /** A line to go to; `at` tells two requests for one line apart. */
    jump: { line: number; at: number } | null;
    onChange: (text: string) => void;
}) {
    const { t, format } = useI18n();
    const words = t.cfg.file;
    const area = useRef<HTMLTextAreaElement>(null);
    const scroller = useRef<HTMLDivElement>(null);
    const lines = useMemo(() => text.split('\n'), [text]);
    const [caret, setCaret] = useState({ line: 1, column: 1 });
    const [query, setQuery] = useState('');
    const [missed, setMissed] = useState(false);

    const place = () => {
        const field = area.current;
        if (field !== null) {
            const before = field.value.slice(0, field.selectionStart);
            setCaret({ line: before.split('\n').length, column: before.length - before.lastIndexOf('\n') });
        }
    };
    const select = (start: number, end: number) => {
        const field = area.current;
        if (field !== null) {
            field.focus({ preventScroll: true });
            field.setSelectionRange(start, end);
            reveal(scroller.current, field.value.slice(0, start).split('\n').length);
            place();
        }
    };

    useEffect(() => {
        const field = area.current;
        if (jump === null || field === null) {
            return;
        }
        const held = field.value.split('\n');
        const line = Math.min(Math.max(jump.line, 1), held.length);
        const start = held.slice(0, line - 1).reduce((sum, each) => sum + each.length + 1, 0);
        field.focus({ preventScroll: true });
        field.setSelectionRange(start, start + (held[line - 1]?.length ?? 0));
        reveal(scroller.current, line);
        setCaret({ line, column: 1 });
    }, [jump]);

    const find = () => {
        const field = area.current;
        if (field === null || query === '') {
            return;
        }
        const held = field.value.toLowerCase();
        const needle = query.toLowerCase();
        const next = held.indexOf(needle, field.selectionEnd);
        const found = next === -1 ? held.indexOf(needle) : next;
        setMissed(found === -1);
        if (found !== -1) {
            select(found, found + needle.length);
        }
    };

    return (
        <section className="card flex min-w-0 flex-col overflow-hidden" aria-label={words.label}>
            <div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-b border-line px-4 py-2.5">
                <span className="font-mono text-ui font-medium text-ink">{label}</span>
                <output className="text-meta text-ink-3 tabular-nums">
                    {words.caret(
                        format.number(caret.line, { useGrouping: false }),
                        format.number(caret.column, { useGrouping: false }),
                    )}
                </output>
                <search className="ml-auto">
                    <form
                        className="flex items-center gap-2"
                        onSubmit={(event) => {
                            event.preventDefault();
                            find();
                        }}
                    >
                        <div className="relative">
                            <Search
                                size={14}
                                strokeWidth={1.5}
                                className="pointer-events-none absolute top-1/2 left-2.5 -translate-y-1/2 text-ink-3"
                                aria-hidden="true"
                            />
                            <input
                                type="search"
                                aria-label={words.find}
                                placeholder={words.find}
                                value={query}
                                aria-invalid={missed || undefined}
                                className={cn(inputClass, 'h-8 w-44 pl-8 text-small max-md:h-11')}
                                onChange={(event) => {
                                    setQuery(event.target.value);
                                    setMissed(false);
                                }}
                            />
                        </div>
                        {missed && <output className="text-meta text-ink-3">{words.notFound}</output>}
                    </form>
                </search>
            </div>
            <div
                ref={scroller}
                className="relative max-h-[64dvh] min-h-80 overflow-auto overscroll-contain bg-console py-2 text-term-fg scrollbar-thin"
            >
                <div className="flex w-max min-w-full">
                    <ol
                        aria-hidden="true"
                        className="sticky left-0 z-10 flex-none bg-console pr-2 pl-3 text-right font-mono text-[12.5px] leading-5 text-ink-3 tabular-nums select-none"
                    >
                        {lines.map((_, index) => {
                            const mark = marks.get(index + 1);
                            return (
                                // oxlint-disable-next-line react/no-array-index-key -- a line is its number
                                <li key={index} className={mark === undefined ? undefined : GUTTER[mark]}>
                                    {index + 1}
                                </li>
                            );
                        })}
                    </ol>
                    <div className="relative min-w-0 flex-1">
                        <pre aria-hidden="true" className={cn(TYPE, 'pointer-events-none')}>
                            {lines.map((line, index) => (
                                // oxlint-disable-next-line react/no-array-index-key -- a line is its number
                                <div key={index} className="h-5">
                                    {coloured(line).map((token) => (
                                        <span
                                            key={token.key}
                                            className={
                                                token.kind === 'secret'
                                                    ? 'rounded-xs bg-surface-3 text-ink-3'
                                                    : TOKEN_CLASS[token.kind]
                                            }
                                        >
                                            {token.text}
                                        </span>
                                    ))}
                                </div>
                            ))}
                        </pre>
                        <textarea
                            ref={area}
                            aria-label={words.text(label)}
                            value={text}
                            wrap="off"
                            spellCheck={false}
                            autoCapitalize="off"
                            autoCorrect="off"
                            autoComplete="off"
                            className={cn(
                                TYPE,
                                'absolute inset-0 size-full resize-none overflow-hidden bg-transparent text-transparent caret-[color:var(--term-fg)] outline-none',
                            )}
                            onChange={(event) => onChange(event.target.value)}
                            onSelect={place}
                            onScroll={(event) => {
                                // Its text is as large as it: the lines scroll, never the field under them.
                                event.currentTarget.scrollTop = 0;
                                event.currentTarget.scrollLeft = 0;
                            }}
                        />
                    </div>
                </div>
            </div>
        </section>
    );
}
